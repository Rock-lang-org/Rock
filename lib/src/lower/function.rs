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
        remapped.relaxed_sized = bounds
            .relaxed_sized
            .iter()
            .map(|parameter| {
                generic_ids
                    .iter()
                    .position(|id| id == parameter)
                    .map(|index| GenericParamId {
                        owner: new_owner,
                        index: index as u32,
                    })
                    .unwrap_or(*parameter)
            })
            .collect();
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn lower_function_sig(&mut self, sig: &ast::FunctionSig) -> HirFunctionSig {
        let signature_id = if let Some(trait_name) = self.current_trait.as_ref() {
            {
                let qualified = format!("{}::{}", trait_name, sig.name.name);
                self.resolver.item_paths.get(&qualified).copied()
            }
        } else {
            self.resolver.item_paths.get(&sig.name.name).copied()
        }
        .unwrap_or_else(|| {
            panic!(
                "missing canonical function signature identity for {}",
                sig.name.name
            )
        });

        self.lower_function_sig_with_id(sig, signature_id)
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
        let mut public_signature_generic_params = Vec::new();
        let mut hidden_signature_generic_params = Vec::new();
        for param in &generic_param_ids {
            if param.owner == generic_owner {
                if let Some(name) = generic_context_params.get(param.index as usize) {
                    if name != "Self" {
                        public_signature_generic_params
                            .push(GenericParamDecl::type_param(*param, name.clone()));
                    } else {
                        hidden_signature_generic_params
                            .push(GenericParamDecl::type_param(*param, name.clone()));
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
            if clause.relaxed {
                if self.language_items.sized.as_ref().map(|item| item.trait_id) != Some(trait_id)
                    || !type_args.is_empty()
                {
                    self.diagnostics.push_with_span(
                        "only the canonical Sized requirement may be relaxed".into(),
                        trait_bound_span,
                    );
                } else if let Type::Generic(parameter) = subject {
                    generic_bounds.relaxed_sized.insert(parameter);
                } else {
                    self.diagnostics.push_with_span(
                        "a relaxed Sized bound requires a generic type parameter".into(),
                        clause.subject.span(),
                    );
                }
                continue;
            }
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

        for parameter in &mut generic_params {
            parameter.maybe_unsized |= generic_bounds.relaxed_sized.contains(&parameter.id);
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

    #[allow(dead_code)]
    pub(crate) fn collect_function_sig(&mut self, fd: &ast::FunctionDecl, function_id: DefId) {
        let name = fd.name.name.clone();

        // Check if there's a standalone signature for this function
        if let Some(sig) = self.items.function_sig(function_id).cloned() {
            // Merge the signature with the function definition
            let mut func = self.lower_function_decl_header_with_sig(fd, &sig, function_id);
            func.is_unsafe = func.is_unsafe || sig.is_unsafe;

            // Register the function type in the scope
            let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope.define_top_level(name.clone(), func_type, false);
            self.items.insert_function(func);

            // Remove the signature since we've now used it
            self.items.remove_function_sig(function_id);
            return;
        }

        let func = self.lower_function_decl_header(fd, function_id);
        // Register the function type in the scope
        let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
        let func_type = Type::function_with_safety(
            param_types,
            func.ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
        );
        self.scope.define_top_level(name.clone(), func_type, false);
        self.items.insert_function(func);
    }

    /// Collect a standalone function signature (without body)
    /// This stores the signature so that a later function definition can use it
    #[allow(dead_code)]
    pub(crate) fn collect_function_signature_only(&mut self, sig: &ast::FunctionSig, id: DefId) {
        let hir_sig = self.lower_function_sig_with_id(sig, id);
        let name = sig.name.name.clone();

        // Store the signature for later use
        // We'll use it when the function body is defined
        self.items.insert_function_sig(hir_sig.clone());
        // Register in scope with the generic type
        let func_type = Type::function_with_safety(
            hir_sig.params.clone(),
            hir_sig.ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(hir_sig.is_unsafe),
        );
        self.scope.define_top_level(name, func_type, false);
    }

    #[allow(dead_code)]
    pub(crate) fn collect_extern(&mut self, sig: &ast::FunctionSig, def_id: DefId) {
        let hir_sig = self.lower_function_sig_with_id(sig, def_id);
        let params = hir_sig.params.clone();
        let ret = hir_sig.ret.clone();
        let name = sig.name.name.clone();

        // Register extern in scope
        let func_type = Type::function_with_safety(
            params.clone(),
            ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(sig.is_unsafe),
        );
        self.scope.define_top_level(name.clone(), func_type, false);

        self.items.insert_extern(HirExtern {
            id: def_id,
            name,
            params,
            ret,
            variadic: false,
            is_unsafe: sig.is_unsafe,
        });
    }

    #[allow(dead_code)]
    pub(crate) fn lower_function_decl_header(
        &mut self,
        fd: &ast::FunctionDecl,
        function_id: DefId,
    ) -> HirFunction {
        self.lower_function_decl_header_with_id(fd, function_id)
    }

    pub(crate) fn lower_function_decl_header_with_id(
        &mut self,
        fd: &ast::FunctionDecl,
        function_id: DefId,
    ) -> HirFunction {
        let lambda = &fd.lambda;
        let func_name = fd.name.name.clone();
        let is_curried = matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried);

        let mut func_type_vars = HashSet::new();
        let mut local_ids = IdGen::<HirLocalId>::new();

        let mut all_params = Vec::new();
        let mut all_param_types = Vec::new();

        // If this is a method, add the implicit self parameter.
        if let Some(self_receiver) = fd.self_receiver {
            let self_param = self.build_self_param_with_local_id(
                self_receiver,
                local_ids.fresh(),
                fd.lambda.span.clone(),
            );
            Self::collect_type_var_ids(&self_param.ty, &mut func_type_vars);
            all_param_types.push(self_param.ty.clone());
            all_params.push(self_param);
        }

        for param in &lambda.parameters {
            let (name, ty, mutable, is_ref) =
                self.lower_param_pattern_at(param, lambda.span.clone());
            Self::collect_type_var_ids(&ty, &mut func_type_vars);
            all_param_types.push(ty.clone());
            let local_id = local_ids.fresh();
            all_params.push(HirParam {
                name,
                local_id,
                ty,
                mutable,
                is_ref,
            });
        }

        let ret_type = if matches!(lambda.arrow_kind, ast::LambdaArrowKind::Unit) {
            Type::Unit
        } else {
            self.engine.fresh_type_var_at(lambda.span.clone())
        };
        Self::collect_type_var_ids(&ret_type, &mut func_type_vars);

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
        let remaining_param_types = all_param_types[outer_param_count..].to_vec();
        let ret_type = if is_curried {
            Self::build_function_type(&remaining_param_types, ret_type, true)
        } else {
            ret_type
        };

        self.function_type_vars.insert(function_id, func_type_vars);
        HirFunction {
            id: function_id,
            name: func_name,
            generic_params: vec![], // filled in later from context
            generic_bounds: crate::hir::HirGenericBounds::new(),
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

    /// Lower a function declaration header with a given signature
    /// This is used when we have a standalone type signature followed by a function definition
    #[allow(dead_code)]
    pub(crate) fn lower_function_decl_header_with_sig(
        &mut self,
        fd: &ast::FunctionDecl,
        sig: &HirFunctionSig,
        id: crate::ids::DefId,
    ) -> HirFunction {
        self.lower_function_decl_header_with_sig_and_id(fd, sig, id)
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
        for mut generic in public_entries {
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
                generic.id = remapped;
                generic_params.push(generic);
            }
        }
        let mut generic_bounds = generic_bounds;
        generic_bounds.retain(|param, _| generic_params.iter().any(|generic| generic.id == *param));

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
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    use crate::ast::{
        Block, FunctionDecl, FunctionSig, Ident, IdentPattern, LambdaArrowKind, LambdaDecl,
        ParseType, ParseTypeInner, Pattern, PatternKind, SelfReceiverMode,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
    use crate::lexer::Span;
    use crate::types::AssociatedTypeKey;

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn unit_signature(name: &str) -> FunctionSig {
        FunctionSig {
            name: ident(name),
            sig: ParseType::Unit(crate::lexer::Span::test()),
            where_clauses: vec![],
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(ParseTypeInner {
            name: name.to_string(),
            generics: Vec::new(),
            span: Span::test(),
        })
    }

    fn generic_identity_signature(name: &str) -> FunctionSig {
        FunctionSig {
            name: ident(name),
            sig: ParseType::Function(vec![named_type("T"), named_type("T")]),
            where_clauses: vec![],
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn binding_pattern(name: &str) -> Pattern {
        Pattern {
            binding: None,
            kind: PatternKind::Ident(IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    fn single_param_function_decl(name: &str, param: &str) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: vec![binding_pattern(param)],
                body: Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn single_param_method_decl(
        name: &str,
        param: &str,
        self_receiver: SelfReceiverMode,
    ) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: vec![binding_pattern(param)],
                body: Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: Some(self_receiver),
            is_unsafe: false,
            exported: false,
        }
    }

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    #[test]
    fn lower_method_signature_injects_shared_self_receiver_type() {
        let mut lowerer = Lowerer::new_for_test();
        let signature_id = def_id(30);
        let sig = FunctionSig {
            name: ident("is_valid"),
            sig: named_type("Bool"),
            where_clauses: vec![],
            self_receiver: Some(SelfReceiverMode::Shared),
            is_unsafe: false,
            exported: false,
        };

        let hir_sig = lowerer.lower_function_sig_with_id(&sig, signature_id);

        let expected_self = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(GenericParamId {
                owner: signature_id,
                index: 0,
            })),
        };
        assert_eq!(hir_sig.params, vec![expected_self]);
        assert_eq!(
            hir_sig.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: signature_id,
                    index: 0,
                },
                "Self",
            )]
        );
        assert_eq!(hir_sig.ret, Type::Bool);
    }

    #[test]
    fn collect_function_sig_uses_explicit_declaration_id() {
        let mut lowerer = Lowerer::new_for_test();
        let declaration = single_param_function_decl("identity", "value");
        let function_id = def_id(31);

        lowerer.collect_function_sig(&declaration, function_id);

        assert_eq!(lowerer.items.function(function_id).unwrap().id, function_id);
        assert_eq!(lowerer.items.function_sigs().count(), 0);
    }

    #[test]
    fn lower_method_signature_injects_mut_and_move_self_receiver_types() {
        let mut lowerer = Lowerer::new_for_test();
        let mut_sig = FunctionSig {
            name: ident("set"),
            sig: ParseType::Function(vec![
                named_type("I64"),
                ParseType::Unit(crate::lexer::Span::test()),
            ]),
            where_clauses: vec![],
            self_receiver: Some(SelfReceiverMode::Mut),
            is_unsafe: false,
            exported: false,
        };
        let move_sig = FunctionSig {
            name: ident("consume"),
            sig: named_type("Bool"),
            where_clauses: vec![],
            self_receiver: Some(SelfReceiverMode::Move),
            is_unsafe: false,
            exported: false,
        };

        let mut_hir = lowerer.lower_function_sig_with_id(&mut_sig, def_id(31));
        let move_hir = lowerer.lower_function_sig_with_id(&move_sig, def_id(32));

        assert!(matches!(
            mut_hir.params[0],
            Type::Reference { mutable: true, .. }
        ));
        assert_eq!(mut_hir.params[1], Type::I64);
        assert!(matches!(move_hir.params[0], Type::Generic(_)));
        assert_eq!(move_hir.ret, Type::Bool);
    }

    #[test]
    fn lower_signature_backed_mut_method_remaps_hidden_self_receiver_generic() {
        let mut lowerer = Lowerer::new_for_test();
        let signature_id = def_id(33);
        let function_id = def_id(34);
        let signature_self = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let hir_sig = HirFunctionSig {
            id: signature_id,
            name: "set".to_string(),
            generic_params: vec![GenericParamDecl::type_param(signature_self, "Self")],
            params: vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Generic(signature_self)),
                },
                Type::I64,
            ],
            ret: Type::Bool,
            generic_bounds: HashMap::new().into(),
            self_receiver: Some(crate::types::ReceiverMode::Mut),
            is_unsafe: false,
        };

        let func = lowerer.lower_function_decl_header_with_sig_and_id(
            &single_param_method_decl("set", "value", SelfReceiverMode::Mut),
            &hir_sig,
            function_id,
        );

        let expected_self = GenericParamId {
            owner: function_id,
            index: 0,
        };
        assert_eq!(
            func.generic_params,
            vec![GenericParamDecl::type_param(expected_self, "Self")]
        );
        assert_eq!(func.params[0].local_id, crate::ids::HirLocalId(0));
        assert!(func.params[0].mutable);
        assert_eq!(
            func.params[0].ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(expected_self)),
            }
        );
        assert_eq!(func.params[1].ty, Type::I64);
    }

    #[test]
    fn generic_param_descriptor_method_keeps_public_name_paired_after_hidden_self() {
        let mut lowerer = Lowerer::new_for_test();
        let signature_id = def_id(35);
        let function_id = def_id(36);
        let signature_self = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let signature_t = GenericParamId {
            owner: signature_id,
            index: 1,
        };
        let hir_sig = HirFunctionSig {
            id: signature_id,
            name: "set".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(signature_self, "Self"),
                GenericParamDecl::type_param(signature_t, "T"),
            ],
            params: vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Generic(signature_self)),
                },
                Type::Generic(signature_t),
            ],
            ret: Type::Generic(signature_t),
            generic_bounds: HashMap::new().into(),
            self_receiver: Some(crate::types::ReceiverMode::Mut),
            is_unsafe: false,
        };

        let func = lowerer.lower_function_decl_header_with_sig_and_id(
            &single_param_method_decl("set", "value", SelfReceiverMode::Mut),
            &hir_sig,
            function_id,
        );

        let expected_t = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let expected_self = GenericParamId {
            owner: function_id,
            index: 1,
        };
        assert_eq!(
            func.generic_params,
            vec![
                GenericParamDecl::type_param(expected_t, "T"),
                GenericParamDecl::type_param(expected_self, "Self"),
            ]
        );
        assert_eq!(
            func.params[0].ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(expected_self)),
            }
        );
        assert_eq!(func.params[1].ty, Type::Generic(expected_t));
        assert_eq!(func.ret_type, Type::Generic(expected_t));
    }

    #[test]
    fn collect_type_var_ids_visits_projection_base_and_trait_args() {
        let base_var = TypeVarId(20);
        let arg_var = TypeVarId(21);
        let trait_id = DefId::new(CrateId(0), LocalDefId(30));
        let ty = Type::Projection {
            ty: Box::new(Type::TypeVar(base_var)),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::TypeVar(arg_var)],
        };
        let mut type_vars = HashSet::new();

        Lowerer::collect_type_var_ids(&ty, &mut type_vars);

        assert!(type_vars.contains(&base_var));
        assert!(type_vars.contains(&arg_var));
    }

    #[test]
    #[should_panic(expected = "missing canonical function signature identity")]
    fn lower_function_sig_panics_without_current_trait_signature_identity() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = DefId::new(CrateId(0), LocalDefId(10));
        lowerer.current_trait = Some("Show".to_string());
        lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
            trait_id,
            vec!["Self".to_string()],
        ));

        lowerer.lower_function_sig(&unit_signature("show"));
    }

    #[test]
    #[should_panic(expected = "missing canonical function signature identity")]
    fn lower_function_sig_rejects_top_level_collision_in_trait_context() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = DefId::new(CrateId(0), LocalDefId(10));
        let top_level_id = DefId::new(CrateId(0), LocalDefId(11));
        lowerer.current_trait = Some("Show".to_string());
        lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
            trait_id,
            vec!["Self".to_string()],
        ));
        lowerer
            .resolver
            .item_paths
            .insert("show".to_string(), top_level_id);

        lowerer.lower_function_sig(&unit_signature("show"));
    }

    #[test]
    fn lower_function_sig_with_id_uses_explicit_id_for_signature_generics() {
        let mut lowerer = Lowerer::new_for_test();
        let stale_resolver_id = def_id(11);
        let signature_id = def_id(12);
        lowerer
            .resolver
            .item_paths
            .insert("id".to_string(), stale_resolver_id);

        let sig =
            lowerer.lower_function_sig_with_id(&generic_identity_signature("id"), signature_id);

        let expected_generic = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        assert_eq!(sig.id, signature_id);
        assert_eq!(
            sig.generic_params,
            vec![GenericParamDecl::type_param(expected_generic, "T")]
        );
        assert_eq!(sig.params, vec![Type::Generic(expected_generic)]);
        assert_eq!(sig.ret, Type::Generic(expected_generic));
    }

    #[test]
    fn lower_function_decl_header_remaps_signature_owned_generics_to_function_id() {
        let mut lowerer = Lowerer::new_for_test();
        let signature_id = def_id(13);
        let function_id = def_id(14);
        let signature_generic = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let hir_sig = HirFunctionSig {
            id: signature_id,
            name: "id".to_string(),
            generic_params: vec![GenericParamDecl::type_param(signature_generic, "T")],
            params: vec![Type::Generic(signature_generic)],
            ret: Type::Generic(signature_generic),
            generic_bounds: HashMap::new().into(),
            self_receiver: None,
            is_unsafe: false,
        };

        let func = lowerer.lower_function_decl_header_with_sig_and_id(
            &single_param_function_decl("id", "value"),
            &hir_sig,
            function_id,
        );

        let expected_generic = GenericParamId {
            owner: function_id,
            index: 0,
        };
        assert_eq!(
            func.generic_params,
            vec![GenericParamDecl::type_param(expected_generic, "T")]
        );
        assert_eq!(func.params[0].ty, Type::Generic(expected_generic));
        assert_eq!(func.ret_type, Type::Generic(expected_generic));
    }

    #[test]
    fn lower_function_decl_header_drops_unused_signature_generics() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(15);
        let phantom_generic = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let hir_sig = HirFunctionSig {
            id: function_id,
            name: "len".to_string(),
            generic_params: vec![GenericParamDecl::type_param(phantom_generic, "T")],
            params: vec![Type::I64],
            ret: Type::I64,
            generic_bounds: HashMap::new().into(),
            self_receiver: None,
            is_unsafe: false,
        };

        let func = lowerer.lower_function_decl_header_with_sig_and_id(
            &single_param_function_decl("len", "value"),
            &hir_sig,
            function_id,
        );

        assert!(func.generic_params.is_empty());
        assert_eq!(func.params[0].ty, Type::I64);
        assert_eq!(func.ret_type, Type::I64);
    }
}

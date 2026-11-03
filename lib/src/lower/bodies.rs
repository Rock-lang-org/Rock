//! Function and impl body lowering

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::ast;
use crate::hir::{substitute_typevars_in_function, HirFunction, HirGenericBounds};
use crate::ids::{CrateId, DefId};
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

use crate::lower::body_context::{BodyLoweringContext, BodyOwner, GenericLoweringContext};
use crate::lower::Lowerer;

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
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
            | "Str"
            | "Char"
            | "()"
    )
}

fn collect_generic_names_from_parse_type<F>(
    ty: &ast::ParseType,
    generic_params: &mut Vec<String>,
    is_known_type_name: &F,
) where
    F: Fn(&str) -> bool,
{
    match ty {
        ast::ParseType::Type(inner) => {
            if inner.generics.is_empty()
                && !is_builtin_type_name(&inner.name)
                && !is_known_type_name(&inner.name)
                && !generic_params.contains(&inner.name)
            {
                generic_params.push(inner.name.clone());
            }
            for generic in &inner.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Application(application) => {
            collect_generic_names_from_parse_type(
                &application.constructor,
                generic_params,
                is_known_type_name,
            );
            for arg in &application.args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            collect_generic_names_from_parse_type(&lambda.body, generic_params, is_known_type_name);
        }
        ast::ParseType::Hole(_) => {}
        ast::ParseType::Associated { base, .. } => {
            for generic in &base.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Array { inner, .. } => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Function(args) | ast::ParseType::Tuple(args) => {
            for arg in args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Unit(_) => {}
    }
}

fn is_known_nominal_type_name(lowerer: &Lowerer, name: &str) -> bool {
    if let Some(id) = lowerer.resolver.resolve_item_or_alias(name) {
        return is_registered_nominal_type_id(lowerer, id);
    }

    lowerer.dependency_resolvers.values().any(|resolver| {
        resolver
            .resolve_item_or_alias(name)
            .is_some_and(|id| is_registered_nominal_type_id(lowerer, id))
    })
}

fn is_registered_nominal_type_id(lowerer: &Lowerer, id: DefId) -> bool {
    lowerer.items.structure(id).is_some() || lowerer.items.enumeration(id).is_some()
}

fn rehome_static_impl_type_generics(
    func: &mut HirFunction,
    type_generics: &[GenericParamDecl],
    impl_id: DefId,
) {
    let mut generic_ids = HashSet::new();
    for param in &func.params {
        param.ty.collect_generic_params(&mut generic_ids);
    }
    func.ret_type.collect_generic_params(&mut generic_ids);
    func.body.ty.collect_generic_params(&mut generic_ids);

    let previous_names: HashMap<GenericParamId, String> = func
        .generic_params
        .iter()
        .map(|param| (param.id, param.name.clone()))
        .collect();

    for generic_id in generic_ids {
        if generic_id.owner == func.id || generic_id.owner == impl_id {
            continue;
        }
        let Some(gen_name) = previous_names.get(&generic_id) else {
            continue;
        };
        if let Some(impl_param) = type_generics.iter().find(|param| &param.name == gen_name) {
            let source = Type::Generic(generic_id);
            let target = Type::Generic(impl_param.id);
            substitute_typevars_in_function(func, &source, &target);
        }
    }
}

fn collect_generic_ids_in_order(ty: &Type, out: &mut Vec<GenericParamId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::Generic(id) = nested {
            if !out.contains(id) {
                out.push(*id);
            }
        }
    });
}

fn function_generic_ids_in_order(func: &HirFunction) -> Vec<GenericParamId> {
    let mut ids = Vec::new();
    for param in &func.params {
        collect_generic_ids_in_order(&param.ty, &mut ids);
    }
    collect_generic_ids_in_order(&func.ret_type, &mut ids);
    collect_generic_ids_in_order(&func.body.ty, &mut ids);
    ids
}

fn sync_static_impl_generic_params_to_ids(
    func: &mut HirFunction,
    type_generics: &[GenericParamDecl],
    impl_id: DefId,
) {
    let previous: HashMap<GenericParamId, GenericParamDecl> = func
        .generic_params
        .iter()
        .map(|param| (param.id, param.clone()))
        .collect();

    let generic_param_ids = function_generic_ids_in_order(func);
    let generic_params = generic_param_ids
        .iter()
        .map(|generic_id| {
            previous
                .get(generic_id)
                .cloned()
                .or_else(|| {
                    (generic_id.owner == impl_id)
                        .then(|| {
                            type_generics
                                .iter()
                                .find(|param| param.id == *generic_id)
                                .cloned()
                        })
                        .flatten()
                })
                .unwrap_or_else(|| {
                    GenericParamDecl::type_param(*generic_id, format!("T{}", generic_id.index))
                })
        })
        .collect();

    func.generic_params = generic_params;
}

impl Lowerer {
    pub(crate) fn impl_type_info(
        &mut self,
        imp: &ast::Impl,
    ) -> (String, Vec<String>, crate::hir::HirImplReceiverPattern) {
        let receiver = imp
            .for_
            .clone()
            .unwrap_or_else(|| imp.name.clone());
        let mut type_generics = Vec::new();
        collect_generic_names_from_parse_type(&receiver, &mut type_generics, &|name| {
            is_known_nominal_type_name(self, name)
        });
        let pattern = crate::hir::HirImplReceiverPattern::Exact(self.lower_parse_type(&receiver));
        (receiver.type_name(), type_generics, pattern)
    }

    pub(crate) fn lower_function_body_qualified(
        &mut self,
        fd: &ast::FunctionDecl,
        function_id: DefId,
        module_prefix: Option<&str>,
    ) {
        let base_name = fd.name.name.clone();
        let name = match module_prefix {
            Some(prefix) => format!("{}::{}", prefix, base_name),
            None => base_name.clone(),
        };

        let func = self.items.function(function_id).cloned();
        let Some(mut func) = func else {
            self.diagnostics.push_toolchain(
                "missing indexed function declaration while lowering body".to_string(),
            );
            return;
        };
        let mut generic_ids = HashSet::new();
        for param in &func.params {
            param.ty.collect_generic_params(&mut generic_ids);
        }
        func.ret_type.collect_generic_params(&mut generic_ids);
        for generic in &func.generic_params {
            generic_ids.insert(generic.id);
        }
        for (generic_id, bounds) in &func.generic_bounds {
            generic_ids.insert(*generic_id);
            for bound in bounds {
                for type_arg in &bound.type_args {
                    type_arg.collect_generic_params(&mut generic_ids);
                }
            }
        }
        if generic_ids
            .into_iter()
            .any(|param| param.owner.crate_id == CrateId(u32::MAX))
        {
            self.diagnostics.push_toolchain(format!(
                "unexpected provisional generic owner while lowering body for {}",
                name
            ));
            return;
        }

        if func.generic_params.is_empty() {
            let mut generic_ids = HashSet::new();
            for param in &func.params {
                param.ty.collect_generic_params(&mut generic_ids);
            }
            func.ret_type.collect_generic_params(&mut generic_ids);
            let mut generic_indices: BTreeSet<u32> = generic_ids
                .iter()
                .filter(|param| param.owner == func.id)
                .map(|param| param.index)
                .collect();
            generic_indices.extend(
                func.generic_bounds
                    .keys()
                    .filter(|param| param.owner == func.id)
                    .map(|param| param.index),
            );
            let default_names = ["T", "U", "V", "W", "X", "Y", "Z"];
            func.generic_params = generic_indices
                .iter()
                .map(|index| {
                    let name = default_names
                        .get(*index as usize)
                        .map(|name| (*name).to_string())
                        .unwrap_or_else(|| format!("T{}", index));
                    GenericParamDecl::type_param(
                        GenericParamId {
                            owner: func.id,
                            index: *index,
                        },
                        name,
                    )
                })
                .collect();
        }

        let body_context = BodyLoweringContext::new(
            name.clone(),
            BodyOwner::Function(func.id),
            (!func.generic_params.is_empty()).then_some(func.id),
            func.generic_params
                .iter()
                .map(|param| param.name.clone())
                .collect(),
            func.generic_bounds.clone(),
            func.is_unsafe,
        );
        let body = self.with_body_context(body_context, |lowerer| {
            lowerer.push_scope();

            // Register parameters in scope
            for (index, param) in func.params.iter_mut().enumerate() {
                let local_id = lowerer.fresh_local_id();
                param.local_id = local_id;
                if let Some(span) = fd
                    .lambda
                    .parameters
                    .get(index)
                    .and_then(Lowerer::pattern_binding_span)
                {
                    lowerer.source_map.insert_local_in_scope(
                        func.id,
                        local_id,
                        span,
                        lowerer.source_scope_stack.last().copied(),
                    );
                }
                lowerer.scope.define_local(
                    param.name.clone(),
                    param.ty.clone(),
                    param.mutable,
                    local_id,
                );
            }

            let body = lowerer.lower_function_body_block(fd);
            lowerer.pop_scope();
            body
        });
        // Unify return type with body type
        if let Err(e) = self.engine.unify(&body.ty, &func.ret_type) {
            let message = format!(
                "In function '{}': return type mismatch: {}",
                name,
                e.render(&self.engine)
            );
            self.diagnostics
                .push_type_with_span(message, fd.lambda.span.clone());
        }
        func.body = body;
        self.resolve_all_types_in_function(&mut func);
        self.items.insert_function(func);
    }

    pub(crate) fn lower_impl_bodies(&mut self, imp: &ast::Impl, impl_id: DefId) {
        let mut impl_generic_params = Vec::new();
        if let Some(for_type) = imp.for_.as_ref() {
            for generic in for_type.generics() {
                collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
                    is_known_nominal_type_name(self, name)
                });
            }
            collect_generic_names_from_parse_type(for_type, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(self, name)
            });
        } else {
            collect_generic_names_from_parse_type(&imp.name, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(self, name)
            });
        }
        for generic in imp.name.generics() {
            collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(self, name)
            });
        }
        let Some(impl_context_params) = self
            .items
            .impl_def(impl_id)
            .map(|hir_impl| hir_impl.type_generics.clone())
        else {
            self.diagnostics.push_with_span(
                "missing indexed impl declaration while lowering bodies".to_string(),
                imp.name.span(),
            );
            return;
        };
        let previous_generic_context = self.generic_context.replace(GenericLoweringContext::new(
            impl_id,
            impl_context_params
                .iter()
                .map(|param| param.name.clone())
                .collect(),
        ));

        // Lower the self type properly (handles &Str, etc.)
        let (type_name, _type_generics, _receiver_pattern) = self.impl_type_info(imp);

        // Process only methods with exact owner-local payloads, in stable DefId order.
        let mut methods = Vec::with_capacity(imp.methods.len());
        for (method_ident, fd) in &imp.methods {
            let method_name = method_ident.name.clone();
            let Some(method_id) = self
                .items
                .impl_def(impl_id)
                .and_then(|hir_impl| hir_impl.methods.get(&method_name))
                .map(|method| method.id)
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed impl method '{}' while lowering bodies",
                    method_name
                ));
                continue;
            };
            methods.push((method_id, method_ident, fd));
        }
        methods.sort_unstable_by_key(|(method_id, _, _)| *method_id);

        for (_, method_ident, fd) in methods {
            let method_name = method_ident.name.clone();

            // Clone the type_generics to avoid holding a borrow
            let type_generics = impl_context_params.clone();

            let Some(existing_func) = self
                .items
                .impl_def(impl_id)
                .and_then(|imp| imp.methods.get(&method_name))
                .cloned()
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed impl method '{}' while lowering bodies",
                    method_name
                ));
                continue;
            };
            {
                let explicit_sig = imp
                    .signatures
                    .iter()
                    .find(|(ident, _)| ident.name == method_name)
                    .map(|(_, sig)| sig);
                let explicit_sig_is_unsafe = explicit_sig.is_some_and(|sig| sig.is_unsafe);
                let mut func = if let Some(sig) = explicit_sig {
                    let hir_sig = self.lower_function_sig_with_id(sig, existing_func.id);
                    self.lower_function_decl_header_with_sig_and_id(fd, &hir_sig, existing_func.id)
                } else {
                    existing_func.clone()
                };
                func.is_unsafe = func.is_unsafe || explicit_sig_is_unsafe;
                // Body lowering reads declared return types through this canonical impl entry.
                self.items
                    .impl_def_mut(impl_id)
                    .unwrap()
                    .methods
                    .insert(method_name.clone(), func.clone());
                let local_id_context = BodyLoweringContext::new(
                    method_name.clone(),
                    BodyOwner::ImplMethod {
                        impl_id,
                        method_id: func.id,
                        method_name: method_name.clone(),
                    },
                    Some(func.id),
                    func.generic_params
                        .iter()
                        .map(|param| param.name.clone())
                        .collect(),
                    HirGenericBounds::new(),
                    fd.is_unsafe || explicit_sig_is_unsafe,
                );
                self.with_body_context(local_id_context, |lowerer| {
                    let source_start = usize::from(func.self_receiver.is_some());
                    for (index, param) in func.params.iter_mut().enumerate() {
                        let local_id = lowerer.fresh_local_id();
                        param.local_id = local_id;
                        let span = if index < source_start {
                            Some(fd.name.span.clone())
                        } else {
                            fd.lambda
                                .parameters
                                .get(index - source_start)
                                .and_then(Lowerer::pattern_binding_span)
                        };
                        if let Some(span) = span {
                            lowerer.source_map.insert_local_in_scope(
                                func.id,
                                local_id,
                                span,
                                lowerer.source_scope_stack.last().copied(),
                            );
                        }
                    }
                });

                self.push_scope();

                // Register generic parameters in scope
                // This allows methods to use T, U, etc. as types
                for generic in &type_generics {
                    self.scope
                        .define(generic.name.clone(), Type::Generic(generic.id), false);
                }

                for generic in &func.generic_params {
                    if !type_generics.iter().any(|param| param.id == generic.id) {
                        self.scope
                            .define(generic.name.clone(), Type::Generic(generic.id), false);
                    }
                }

                // If self is injected, define self with the right type.
                if let Some(self_receiver) = func.self_receiver {
                    let concrete_self_ty =
                        match &self.items.impl_def(impl_id).unwrap().receiver_pattern {
                            crate::hir::HirImplReceiverPattern::Exact(ty)
                            | crate::hir::HirImplReceiverPattern::Constructor(ty) => ty.clone(),
                            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                                Type::Slice(Box::new(element.clone()))
                            }
                        };
                    let actual_self_ty = match (self_receiver, concrete_self_ty) {
                        (
                            crate::types::ReceiverMode::Mut,
                            Type::Reference {
                                mutable: true,
                                inner,
                            },
                        ) => Type::Reference {
                            mutable: true,
                            inner,
                        },
                        (crate::types::ReceiverMode::Shared, concrete_self_ty) => Type::Reference {
                            mutable: false,
                            inner: Box::new(concrete_self_ty),
                        },
                        (crate::types::ReceiverMode::Mut, concrete_self_ty) => Type::Reference {
                            mutable: true,
                            inner: Box::new(concrete_self_ty),
                        },
                        (crate::types::ReceiverMode::Move, concrete_self_ty) => concrete_self_ty,
                    };
                    let _ = self.engine.unify(&func.params[0].ty, &actual_self_ty);
                    func.params[0].ty = actual_self_ty.clone();
                    self.scope.define_local(
                        "self".to_string(),
                        actual_self_ty,
                        matches!(self_receiver, crate::types::ReceiverMode::Mut),
                        func.params[0].local_id,
                    );
                }

                // Register parameters in scope
                let start = if func.self_receiver.is_some() { 1 } else { 0 };
                for param in &func.params[start..] {
                    self.scope.define_local(
                        param.name.clone(),
                        param.ty.clone(),
                        param.mutable,
                        param.local_id,
                    );
                }

                // Reuse the collected bounds, including constructor predicates.
                // Re-parsing only named type subjects here loses HKT impl bounds.
                let mut impl_bounds = self.items.impl_def(impl_id).unwrap().bounds.clone();
                for (param, bounds) in &func.generic_bounds {
                    impl_bounds
                        .entry(*param)
                        .or_default()
                        .extend(bounds.clone());
                }
                impl_bounds
                    .predicates
                    .extend(func.generic_bounds.predicates.clone());
                // Inject implicit Sized bound for all type params
                if let Some(sized_trait_id) = self
                    .language_items
                    .sized
                    .as_ref()
                    .map(|items| items.trait_id)
                {
                    for generic in &type_generics {
                        if generic.kind != crate::type_services::kind::Kind::Type {
                            continue;
                        }
                        impl_bounds
                            .entry(generic.id)
                            .or_insert_with(Vec::new)
                            .push(TraitBound {
                                trait_id: sized_trait_id,
                                type_args: Vec::new(),
                            });
                    }
                    for generic in &func.generic_params {
                        if generic.kind != crate::type_services::kind::Kind::Type {
                            continue;
                        }
                        impl_bounds
                            .entry(generic.id)
                            .or_insert_with(Vec::new)
                            .push(TraitBound {
                                trait_id: sized_trait_id,
                                type_args: Vec::new(),
                            });
                    }
                }
                let current_struct_impl =
                    crate::lower::resolution::LowerResolutionContext::new(self)
                        .resolve_module_alias_or_item_id(&type_name)
                        .filter(|id| self.items.structure(*id).is_some())
                        .and_then(|id| self.resolver.item_names_by_id.get(&id).cloned());
                let prev_impl_id = self.current_impl_id;
                self.current_impl_id = Some(impl_id);
                let has_constructor_method_generic = func
                    .generic_params
                    .iter()
                    .any(|param| !matches!(param.kind, crate::type_services::kind::Kind::Type));
                let (generic_owner, generic_params) = if !has_constructor_method_generic {
                    (
                        Some(impl_id),
                        type_generics
                            .iter()
                            .map(|param| param.name.clone())
                            .collect(),
                    )
                } else {
                    (
                        Some(func.id),
                        func.generic_params
                            .iter()
                            .filter(|param| param.id.owner == func.id)
                            .map(|param| param.name.clone())
                            .collect(),
                    )
                };
                let mut body_context = BodyLoweringContext::new(
                    method_name.clone(),
                    BodyOwner::ImplMethod {
                        impl_id,
                        method_id: func.id,
                        method_name: method_name.clone(),
                    },
                    generic_owner,
                    generic_params,
                    impl_bounds,
                    fd.is_unsafe || explicit_sig_is_unsafe,
                );
                for generic in type_generics.iter().chain(&func.generic_params) {
                    body_context.register_generic_param(generic.name.clone(), generic.id);
                }
                body_context
                    .seed_after_existing_locals(func.params.iter().map(|param| param.local_id));
                let body = self.with_body_context(body_context, |lowerer| {
                    lowerer.with_current_struct_impl(current_struct_impl, |lowerer| {
                        lowerer.lower_function_body_block(fd)
                    })
                });
                self.current_impl_id = prev_impl_id;

                if let Err(e) = self.engine.unify(&body.ty, &func.ret_type) {
                    let message = format!(
                        "In method '{}.{}': return type mismatch: {}",
                        type_name,
                        method_name,
                        e.render(&self.engine)
                    );
                    self.diagnostics
                        .push_type_with_span(message, fd.lambda.span.clone());
                }
                func.body = body;
                self.resolve_all_types_in_function(&mut func);
                self.pop_scope();

                // For non-self (associated) functions, inherit impl type_generics as
                // generic_params so the monomorphizer can specialize them from call-site types.
                if func.self_receiver.is_none() {
                    for generic in &type_generics {
                        if !func
                            .generic_params
                            .iter()
                            .any(|param| param.id == generic.id)
                        {
                            func.generic_params.push(generic.clone());
                        }
                    }
                }

                if func.self_receiver.is_none() {
                    rehome_static_impl_type_generics(&mut func, &type_generics, impl_id);

                    let _ = self.engine.unify(&func.body.ty, &func.ret_type);
                    self.resolve_all_types_in_function(&mut func);
                    sync_static_impl_generic_params_to_ids(&mut func, &type_generics, impl_id);
                }

                // Keep the source-style alias for lookup while the method payload remains
                // owned by its impl.
                if func.self_receiver.is_none() {
                    let associated_name = format!("{}::{}", type_name, method_name);
                    self.current_def_ids.insert(func.id);
                    self.resolver
                        .item_paths
                        .insert(associated_name.clone(), func.id);
                    self.resolver
                        .item_names_by_id
                        .entry(func.id)
                        .or_insert_with(|| associated_name.clone());
                }

                self.items
                    .impl_def_mut(impl_id)
                    .unwrap()
                    .methods
                    .insert(method_name.clone(), func);
            }
        }

        self.generic_context = previous_generic_context;
    }
}

#[cfg(test)]
mod tests;

impl Lowerer {
    fn lower_function_body_block(&mut self, fd: &ast::FunctionDecl) -> crate::hir::HirBlock {
        if fd.lambda.arrow_kind == ast::LambdaArrowKind::Curried && fd.lambda.parameters.len() > 1 {
            let span = fd.name.span.clone();
            let remaining_params = &fd.lambda.parameters[1..];
            let curried_signature = self.current_body_return_type().and_then(|mut ty| {
                let mut parameter_types = Vec::with_capacity(remaining_params.len());
                for _ in remaining_params {
                    ty = match self.engine.resolve(&ty) {
                        Type::Function { params, ret, .. } if params.len() == 1 => {
                            parameter_types.push(params[0].clone());
                            *ret
                        }
                        _ => return None,
                    };
                }
                Some((parameter_types, ty))
            });
            let inner = if let Some((parameter_types, return_type)) = curried_signature {
                self.with_body_return_type(return_type, |lowerer| {
                    lowerer.lower_curried_lambda_stage_with_expected_params(
                        remaining_params,
                        &fd.lambda.body,
                        &span,
                        Some(&parameter_types),
                    )
                })
            } else {
                self.lower_curried_lambda_stage(remaining_params, &fd.lambda.body, &span)
            };

            crate::hir::HirBlock {
                ty: inner.ty.clone(),
                stmts: vec![crate::hir::HirStmt::Expr(inner)],
            }
        } else {
            self.lower_lambda_body(&fd.lambda)
        }
    }
}

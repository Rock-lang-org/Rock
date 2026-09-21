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
            .unwrap_or_else(|| ast::ParseType::Type(imp.name.clone()));
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
            for generic in &imp.name.generics {
                collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
                    is_known_nominal_type_name(self, name)
                });
            }
        }
        for generic in &imp.name.generics {
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
                imp.name.span.clone(),
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
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::ast::{
        Block, Expression, FunctionDecl, Ident, IdentOrNumber, IdentOrType, IdentifierPath,
        LambdaArrowKind, LambdaDecl, Literal, LiteralKind, Operand, ParseTypeInner, PrimaryExpr,
        SecondaryExpr, SelfReceiverMode, Statement, TypePath, UnaryExpr,
    };
    use crate::hir::{
        HirBlock, HirEnum, HirExprKind, HirField, HirFunction, HirImpl, HirImplOwner,
        HirImplReceiverPattern, HirParam, HirStruct,
    };
    use crate::ids::{FieldId, LocalDefId, VariantId};
    use crate::lexer::Span;

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn parse_type(name: &str, generics: Vec<ast::ParseType>) -> ast::ParseType {
        ast::ParseType::Type(ParseTypeInner {
            name: name.to_string(),
            generics,
            span: Span::test(),
        })
    }

    fn empty_function_decl(name: &str) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: Vec::new(),
                body: Block {
                    statements: Vec::new(),
                },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn int_expr(value: u64) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn string_instance_expr() -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Instance(crate::ast::Instance {
                name: TypePath {
                    path: vec![IdentOrType::Ident(ident("String"))],
                },
                fields: HashMap::from([(ident("ptr"), int_expr(1))]),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn self_field_expr(field: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(ident("self"))],
            }),
            secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(ident(field)))]),
            type_annotation: None,
        }))
    }

    fn private_field_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "ptr".to_string(),
                ty: Type::I64,
                public: false,
            }],
        }
    }

    fn unit_enum(id: DefId, name: &str, generic_params: Vec<GenericParamDecl>) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params,
            variants: vec![crate::hir::HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: crate::hir::HirVariantFields::Unit,
            }],
        }
    }

    fn empty_impl_function(name: &str, id: DefId, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn shared_method_placeholder(
        name: &str,
        id: DefId,
        self_ty: Type,
        ret_type: Type,
    ) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: self_ty,
                mutable: false,
                is_ref: false,
            }],
            ret_type,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(crate::types::ReceiverMode::Shared),
            is_unsafe: false,
        }
    }

    fn function_with_generic_owner(name: &str, function_id: DefId, owner: DefId) -> HirFunction {
        let generic = GenericParamId { owner, index: 0 };

        HirFunction {
            id: function_id,
            name: name.to_string(),
            generic_params: vec![GenericParamDecl::type_param(generic, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(generic),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn rehome_static_impl_type_generics_keeps_function_owned_name_collision() {
        let function_id = def_id(10);
        let impl_id = def_id(20);
        let function_generic = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let mut func = HirFunction {
            id: function_id,
            name: "id".to_string(),
            generic_params: vec![GenericParamDecl::type_param(function_generic, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(function_generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(function_generic),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Generic(function_generic),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        rehome_static_impl_type_generics(
            &mut func,
            &[GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            impl_id,
        );

        assert_eq!(
            func.generic_params,
            vec![GenericParamDecl::type_param(function_generic, "T")]
        );
        assert_eq!(func.params[0].ty, Type::Generic(function_generic));
        assert_eq!(func.ret_type, Type::Generic(function_generic));
    }

    #[test]
    fn rehome_static_impl_type_generics_keeps_existing_impl_owned_id() {
        let function_id = def_id(10);
        let impl_id = def_id(20);
        let function_generic_u = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let impl_generic_t = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let mut func = HirFunction {
            id: function_id,
            name: "pair".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(function_generic_u, "U"),
                GenericParamDecl::type_param(impl_generic_t, "T"),
            ],
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(function_generic_u),
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "boxed".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(impl_generic_t),
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: Type::Generic(impl_generic_t),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Generic(impl_generic_t),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        rehome_static_impl_type_generics(
            &mut func,
            &[GenericParamDecl::type_param(impl_generic_t, "T")],
            impl_id,
        );

        assert_eq!(func.params[1].ty, Type::Generic(impl_generic_t));
        assert_eq!(func.ret_type, Type::Generic(impl_generic_t));
        assert_eq!(func.body.ty, Type::Generic(impl_generic_t));
    }

    #[test]
    fn sync_static_impl_generic_params_keeps_ids_and_names_cardinality_equal() {
        let function_id = def_id(10);
        let impl_id = def_id(20);
        let function_generic = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let impl_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let mut func = HirFunction {
            id: function_id,
            name: "pair".to_string(),
            generic_params: vec![GenericParamDecl::type_param(function_generic, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(function_generic),
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "boxed".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(impl_generic),
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: Type::Generic(function_generic),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Generic(function_generic),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };

        sync_static_impl_generic_params_to_ids(
            &mut func,
            &[GenericParamDecl::type_param(impl_generic, "T")],
            impl_id,
        );

        assert_eq!(
            func.generic_params,
            vec![
                GenericParamDecl::type_param(function_generic, "T"),
                GenericParamDecl::type_param(impl_generic, "T"),
            ]
        );
    }

    #[test]
    fn lower_impl_bodies_resolves_current_struct_impl_by_prepared_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(40);
        let impl_id = def_id(41);
        let function_id = def_id(42);
        lowerer.items.insert_structure(private_field_struct(
            struct_id,
            "stdlib::string_type::String",
        ));
        lowerer
            .resolver
            .scoped_module_aliases
            .entry("stdlib::string_type".to_string())
            .or_default()
            .insert("String".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "stdlib::string_type::String".to_string());
        lowerer
            .modules
            .replace_qualified_module_prefix(Some("stdlib::string_type"));
        let function = empty_impl_function(
            "new",
            function_id,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
        );
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("String".to_string()),
                type_name: "String".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("new".to_string(), function.clone())]),
            })
            .unwrap();
        let ast_impl = crate::ast::Impl {
            name: crate::ast::ParseTypeInner {
                name: "String".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(
                ident("new"),
                FunctionDecl {
                    name: ident("new"),
                    lambda: LambdaDecl {
                        parameters: Vec::new(),
                        body: Block {
                            statements: vec![Statement::Expression(string_instance_expr())],
                        },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, impl_id);

        assert!(
            lowerer
                .errors()
                .iter()
                .all(|error| !error.message.contains("private fields")),
            "same-struct construction should not report private fields: {:?}",
            lowerer.errors()
        );
    }

    #[test]
    fn lower_impl_bodies_inherent_impl_does_not_match_trait_impl_slot() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(70);
        let inherent_impl_id = def_id(71);
        let trait_id = def_id(72);
        let trait_impl_id = def_id(73);
        let inherent_function_id = def_id(74);
        let trait_function_id = def_id(75);
        lowerer
            .items
            .insert_structure(private_field_struct(struct_id, "Thing"));
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Thing".to_string());
        let inherent_function = empty_impl_function("value", inherent_function_id, Type::I64);
        let trait_function = empty_impl_function("value", trait_function_id, Type::Bool);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: inherent_impl_id,
                owner: HirImplOwner::Named("Thing".to_string()),
                type_name: "Thing".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), inherent_function)]),
            })
            .unwrap();
        lowerer
            .items
            .insert_impl(HirImpl {
                id: trait_impl_id,
                owner: HirImplOwner::Named("Thing".to_string()),
                type_name: "Thing".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Readable".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), trait_function)]),
            })
            .unwrap();
        let ast_impl = crate::ast::Impl {
            name: ParseTypeInner {
                name: "Thing".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(
                ident("value"),
                FunctionDecl {
                    name: ident("value"),
                    lambda: LambdaDecl {
                        parameters: Vec::new(),
                        body: Block {
                            statements: vec![Statement::Expression(int_expr(1))],
                        },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, inherent_impl_id);

        assert_eq!(
            lowerer.items.impl_def(inherent_impl_id).unwrap().methods["value"]
                .body
                .ty,
            Type::I64
        );
        assert_eq!(
            lowerer.items.impl_def(trait_impl_id).unwrap().methods["value"]
                .body
                .ty,
            Type::Unit
        );
    }

    #[test]
    fn lower_impl_bodies_context_owner_matches_trait_args() {
        let mut lowerer = Lowerer::new_for_test();
        let box_id = def_id(80);
        let trait_id = def_id(81);
        let matching_impl_id = def_id(82);
        let other_impl_id = def_id(83);
        let function_id = def_id(84);
        let matching_generic = GenericParamId {
            owner: matching_impl_id,
            index: 0,
        };
        let other_generic = GenericParamId {
            owner: other_impl_id,
            index: 0,
        };
        lowerer.items.insert_structure(HirStruct {
            id: box_id,
            name: "Box".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: box_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Box".to_string(), box_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(box_id, "Box".to_string());
        lowerer
            .resolver
            .item_paths
            .insert("Convert".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Convert".to_string());
        lowerer
            .items
            .insert_impl(HirImpl {
                id: matching_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: vec![GenericParamDecl::type_param(matching_generic, "T")],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: box_id,
                    args: vec![Type::Generic(matching_generic)],
                }),
                trait_name: Some("Convert".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::Bool],
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "take".to_string(),
                    empty_impl_function("take", function_id, Type::Unit),
                )]),
            })
            .unwrap();
        lowerer
            .items
            .insert_impl(HirImpl {
                id: other_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: vec![GenericParamDecl::type_param(other_generic, "T")],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: box_id,
                    args: vec![Type::Generic(other_generic)],
                }),
                trait_name: Some("Convert".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();
        let ast_impl = crate::ast::Impl {
            name: ParseTypeInner {
                name: "Convert".to_string(),
                generics: vec![parse_type("Bool", Vec::new())],
                span: Span::test(),
            },
            for_: Some(parse_type("Box", vec![parse_type("T", Vec::new())])),
            associated_types: Vec::new(),
            methods: HashMap::from([(ident("take"), empty_function_decl("take"))]),
            signatures: HashMap::from([(
                ident("take"),
                crate::ast::FunctionSig {
                    name: ident("take"),
                    sig: ast::ParseType::Function(vec![parse_type("T", Vec::new())]),
                    where_clauses: Vec::new(),
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, matching_impl_id);

        assert_eq!(
            lowerer.items.impl_def(matching_impl_id).unwrap().methods["take"].ret_type,
            Type::Generic(matching_generic)
        );
        assert!(!lowerer
            .items
            .impl_def(other_impl_id)
            .unwrap()
            .methods
            .contains_key("take"));
    }

    #[test]
    fn is_known_nominal_type_name_recognizes_dependency_alias_id() {
        let mut lowerer = Lowerer::new_for_test();
        let dep_struct_id = DefId::new(CrateId(2), LocalDefId(10));
        lowerer.items.insert_structure(HirStruct {
            id: dep_struct_id,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        let mut dep_resolver = crate::collect::resolver::ResolverTables::default();
        dep_resolver
            .import_aliases
            .insert("Widget".to_string(), dep_struct_id);
        dep_resolver
            .item_names_by_id
            .insert(dep_struct_id, "dep::Widget".to_string());
        lowerer
            .dependency_resolvers
            .insert("dep".to_string(), dep_resolver);

        assert!(is_known_nominal_type_name(&lowerer, "Widget"));
    }

    #[test]
    fn lower_impl_bodies_rewraps_shared_signature_self_as_concrete_receiver() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(50);
        let impl_id = def_id(51);
        let function_id = def_id(52);
        lowerer
            .items
            .insert_structure(private_field_struct(struct_id, "Point"));
        lowerer
            .resolver
            .item_paths
            .insert("Point".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Point".to_string());
        let signature_self = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let function = shared_method_placeholder(
            "get",
            function_id,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::I64,
        );
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Point".to_string()),
                type_name: "Point".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("get".to_string(), function.clone())]),
            })
            .unwrap();
        let ast_impl = crate::ast::Impl {
            name: ParseTypeInner {
                name: "Point".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(
                ident("get"),
                FunctionDecl {
                    name: ident("get"),
                    lambda: LambdaDecl {
                        parameters: Vec::new(),
                        body: Block {
                            statements: vec![Statement::Expression(self_field_expr("ptr"))],
                        },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: Some(SelfReceiverMode::Shared),
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, impl_id);

        let lowered = &lowerer.items.impl_def(impl_id).unwrap().methods["get"];
        assert_eq!(
            lowered.params[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
            }
        );
        assert_eq!(lowered.body.ty, Type::I64);
        let Some(crate::hir::HirStmt::Expr(expr)) = lowered.body.stmts.first() else {
            panic!("expected lowered field expression");
        };
        match &expr.kind {
            HirExprKind::FieldAccess(_, _, Some(location)) => {
                assert_eq!(location.owner, struct_id);
            }
            other => panic!("expected located field access, got {other:?}"),
        }
    }

    #[test]
    fn lower_impl_bodies_rewraps_enum_self_as_enum_receiver() {
        let mut lowerer = Lowerer::new_for_test();
        let enum_id = def_id(60);
        let impl_id = def_id(61);
        let function_id = def_id(62);
        lowerer
            .items
            .insert_enumeration(unit_enum(enum_id, "Option", Vec::new()));
        lowerer
            .resolver
            .item_paths
            .insert("Option".to_string(), enum_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(enum_id, "Option".to_string());
        let signature_self = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let function = shared_method_placeholder(
            "is_some",
            function_id,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::Bool,
        );
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Enum {
                    id: enum_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("is_some".to_string(), function.clone())]),
            })
            .unwrap();
        let ast_impl = crate::ast::Impl {
            name: ParseTypeInner {
                name: "Option".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(
                ident("is_some"),
                FunctionDecl {
                    name: ident("is_some"),
                    lambda: LambdaDecl {
                        parameters: Vec::new(),
                        body: Block {
                            statements: vec![Statement::Expression(Expression::UnaryExpr(
                                UnaryExpr::PrimaryExpr(PrimaryExpr {
                                    operand: Operand::Literal(Literal {
                                        kind: LiteralKind::Bool(true),
                                        span: Span::test(),
                                    }),
                                    secondaries: None,
                                    type_annotation: None,
                                }),
                            ))],
                        },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: Some(SelfReceiverMode::Shared),
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, impl_id);

        let lowered = &lowerer.items.impl_def(impl_id).unwrap().methods["is_some"];
        assert_eq!(
            lowered.params[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Enum {
                    id: enum_id,
                    args: Vec::new(),
                }),
            }
        );
    }

    #[test]
    fn lower_function_body_reports_provisional_generic_owner_without_mutation() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(10);
        let provisional_owner = DefId::new(CrateId(u32::MAX), LocalDefId(99));
        lowerer.items.insert_function(function_with_generic_owner(
            "id",
            function_id,
            provisional_owner,
        ));
        lowerer
            .resolver
            .item_paths
            .insert("id".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "id".to_string());

        lowerer.lower_function_body_qualified(&empty_function_decl("id"), function_id, None);

        let function = lowerer.items.function(function_id).unwrap();
        assert_eq!(function.body.ty, Type::Unit);
        assert_eq!(function.generic_params[0].id.owner, provisional_owner);
        assert!(!lowerer.diagnostics.is_empty());
    }

    #[test]
    fn lower_function_body_reports_missing_indexed_function_payload() {
        let mut lowerer = Lowerer::new_for_test();

        lowerer.lower_function_body_qualified(&empty_function_decl("missing"), def_id(90), None);

        assert!(lowerer.errors().iter().any(|error| error
            .message
            .contains("missing indexed function declaration while lowering body")));
    }

    #[test]
    fn lower_impl_bodies_reports_missing_indexed_method_payload() {
        let mut lowerer = Lowerer::new_for_test();
        let impl_id = def_id(91);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Missing".to_string()),
                type_name: "Missing".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::I64),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();
        let ast_impl = ast::Impl {
            name: ParseTypeInner {
                name: "Missing".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(ident("value"), empty_function_decl("value"))]),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        lowerer.lower_impl_bodies(&ast_impl, impl_id);

        assert!(lowerer.errors().iter().any(|error| error
            .message
            .contains("missing indexed impl method 'value' while lowering bodies")));
    }
}

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

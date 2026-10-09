use std::collections::HashMap;

use super::hir_types::{
    hir_function_is_codegen_concrete, HirBlock, HirCallTarget, HirExpr, HirExprKind, HirFunction,
    HirMethodCallTarget, HirStmt, HirVarRef, HirVarTarget,
};
use crate::ids::DefId;
#[cfg(test)]
use crate::types::ReceiverMode;
use crate::types::Type;

use super::{InstanceKey, InstanceOrigin, Monomorphizer};

fn has_measured_storage(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |ty| {
        matches!(
            ty,
            Type::StructRepresentation { .. } | Type::EnumRepresentation { .. }
        ) || matches!(ty, Type::Function { captures, .. } if !captures.is_empty())
    })
}

impl Monomorphizer {
    pub(super) fn register_effective_trait_methods(
        &mut self,
        program: &super::hir_types::HirProgram,
    ) {
        self.effective_trait_methods
            .extend(program.indexes.effective_trait_methods.iter().map(
                |(&(impl_id, trait_method_id), &impl_method_id)| {
                    ((impl_id, trait_method_id), impl_method_id)
                },
            ));
        for (&(_, trait_method_id), &impl_method_id) in &program.indexes.effective_trait_methods {
            if trait_method_id != impl_method_id {
                self.source_map.remap_owner(trait_method_id, impl_method_id);
            }
        }
    }

    pub(super) fn method_for_selected_target<'a>(
        &self,
        imp: &'a super::hir_types::HirImpl,
        target: &HirMethodCallTarget,
    ) -> Option<&'a HirFunction> {
        let method_id = match target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod { method_id, .. } => method_id,
            crate::hir::HirSelectedMethodTarget::TraitMethod { member_id, .. } => self
                .effective_trait_methods
                .get(&(imp.id, member_id))
                .copied()?,
        };
        imp.methods.values().find(|method| method.id == method_id)
    }

    #[cfg(test)]
    fn lookup_self_receiver(&self, target: Option<&HirMethodCallTarget>) -> Option<ReceiverMode> {
        let target = target?;
        if let Some(impl_id) = target.impl_id() {
            let method_id = target.method_id()?;
            return self
                .generic_impls
                .values()
                .chain(self.trait_impls.values().flat_map(|impls| impls.iter()))
                .find(|imp| imp.id == impl_id)
                .and_then(|imp| imp.methods.values().find(|method| method.id == method_id))
                .and_then(|method| method.self_receiver);
        }

        let trait_id = target.trait_id()?;
        let member_id = target.method_id()?;
        self.trait_impls
            .values()
            .flat_map(|impls| impls.iter())
            .filter(|imp| imp.trait_id == Some(trait_id))
            .find_map(|imp| imp.methods.values().find(|method| method.id == member_id))
            .and_then(|method| method.self_receiver)
    }

    fn zero_substitution_function_instance(&self, id: DefId) -> Option<crate::mono::InstanceId> {
        self.instances
            .get(&InstanceKey::new(InstanceOrigin::Function(id), Vec::new()))
    }

    pub(super) fn zero_substitution_callable_instance(
        &self,
        id: DefId,
    ) -> Option<crate::mono::InstanceId> {
        self.zero_substitution_function_instance(id)
    }

    pub(super) fn process(
        &mut self,
        mut program: super::hir_types::HirProgram,
    ) -> super::hir_types::HirProgram {
        self.language_items = program.language_items.clone();
        self.object_traits
            .extend(program.traits.iter().map(|(id, tr)| (*id, tr.clone())));
        let canonical_names_by_id = Self::canonical_names_from_indexes(&program);
        self.drop_trait_id = program
            .language_items
            .drop
            .as_ref()
            .map(|items| items.trait_id);
        self.drop_method_id = program
            .language_items
            .drop
            .as_ref()
            .map(|items| items.method_id);
        self.register_effective_trait_methods(&program);
        self.register_nominal_field_types(&program);

        let mut impls = program
            .impls_in_order()
            .map(|(_, imp)| imp.clone())
            .collect::<Vec<_>>();
        impls.sort_by_key(|imp| imp.id);
        self.collect_impls(&impls);

        let mut all_funcs = program
            .functions_by_id()
            .map(|(id, name, func)| (id, name.to_string(), func.clone()))
            .collect::<Vec<_>>();
        all_funcs.sort_by_key(|(id, _, _)| *id);
        program.functions.clear();
        for (_, name, func) in all_funcs {
            let is_entrypoint = name == "main" && func.generic_params.is_empty();
            if is_entrypoint
                || (func.generic_params.is_empty() && hir_function_is_codegen_concrete(&func))
                || Self::requires_scoped_entry(&func)
            {
                self.register_concrete_function(name.clone(), func);
            } else {
                self.register_generic_function(name, func);
            }
        }

        // Retain accepted definition requirements before processing/removing
        // concrete definitions or substituting any caller instance.
        self.retained_callable_inputs = self.definition_callable_inputs();
        if let Err(error) = self.prepare_polymorphic_members(&program) {
            self.diagnostics.push(
                crate::diagnostic::Diagnostic::for_internal(error)
                    .with_code(crate::diagnostic::DiagnosticCode::Mono),
            );
        }
        let mut function_ids = self.concrete_functions.keys().copied().collect::<Vec<_>>();
        function_ids.sort();
        for id in function_ids {
            self.process_concrete_definition(id);
        }

        let mut impl_ids = program.impls.keys().copied().collect::<Vec<_>>();
        impl_ids.sort();
        for impl_id in impl_ids {
            let Some(imp) = program.impls.get_mut(&impl_id) else {
                continue;
            };
            let method_names = Self::method_names_in_id_order(&imp.methods);
            for name in method_names {
                if let Some(func) = imp.methods.remove(&name) {
                    let processed = self.process_function(func);
                    if Self::should_register_method_instance(&processed) {
                        self.register_impl_method_instance(imp, &name, &processed);
                    }
                    imp.methods.insert(name, processed);
                }
            }
        }

        self.process_and_register_concrete_trait_defaults(&mut program);
        self.monomorphize_known_drop_types();

        program.functions.clear();
        program.names.functions_by_name.clear();
        let mut functions = std::mem::take(&mut self.concrete_functions)
            .into_iter()
            .collect::<Vec<_>>();
        functions.sort_by_key(|(id, _)| *id);
        for (id, func) in functions {
            let name = self
                .function_names_by_id
                .get(&id)
                .expect("registered function must retain canonical display metadata")
                .clone();
            self.register_processed_function_instance(&name, &func);
            program.names.functions_by_name.insert(name, id);
            program.functions.insert(id, func);
        }
        program.rebuild_indexes_with_canonical_names(&canonical_names_by_id);

        program
    }

    pub(super) fn canonical_names_from_indexes(
        program: &super::hir_types::HirProgram,
    ) -> HashMap<DefId, String> {
        let mut names = program.indexes.functions_by_id.clone();
        names.extend(program.indexes.structs_by_id.clone());
        names.extend(program.indexes.enums_by_id.clone());
        names.extend(program.indexes.traits_by_id.clone());
        names
    }

    fn process_concrete_definition(&mut self, id: DefId) {
        if self.processed_concrete_functions.contains(&id) {
            return;
        }
        // Removal is the recursion guard: an active definition cannot certify
        // its own unfinished return representation through a cycle.
        if let Some(function) = self.concrete_functions.remove(&id) {
            let processed = self.process_function(function);
            self.concrete_functions.insert(id, processed);
            self.processed_concrete_functions.insert(id);
        }
    }

    pub(super) fn process_function(&mut self, mut func: HirFunction) -> HirFunction {
        let previous_owner = self.current_function_owner;
        self.current_function_owner = Some(func.id);
        let previous_var_types = std::mem::take(&mut self.var_types);
        self.process_params(Some(func.id), &mut func.params);
        func.ret_type = self.substitute_generics(&func.ret_type);
        for ty in func
            .params
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(&func.ret_type))
        {
            if let Err(message) = self.ensure_payload_schemas(ty, &mut Default::default()) {
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::for_internal(message)
                        .with_code(crate::diagnostic::DiagnosticCode::Mono),
                );
            }
        }
        match self.lower_scoped_function(&mut func) {
            Ok(true) => {}
            Ok(false) => self.process_block(&mut func.body),
            Err((span, message)) => {
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::new(message, span)
                        .with_code(crate::diagnostic::DiagnosticCode::Mono),
                );
            }
        }
        if let Some(actual) =
            crate::type_services::callable::value_representation(&func.ret_type, &func.body.ty)
        {
            if block_returns_agree(&func.body, &actual) {
                func.ret_type = actual;
            }
        }
        self.current_function_owner = previous_owner;
        self.var_types = previous_var_types;
        func
    }

    pub(super) fn process_params(
        &mut self,
        owner: Option<DefId>,
        params: &mut [crate::hir::HirParam],
    ) {
        for param in params {
            param.ty = self.substitute_generics(&param.ty);
            let param_span =
                owner.and_then(|owner| self.source_map.local_span(owner, param.local_id).cloned());
            self.monomorphize_drop_for_type(&param.ty, param_span);
            let ty_id = self.intern_type(&param.ty);
            self.var_types.insert(param.local_id, ty_id);
        }
    }

    pub(super) fn process_block(&mut self, block: &mut HirBlock) {
        let enclosing_var_types = self.var_types.clone();
        block.ty = self.substitute_generics(&block.ty);
        for stmt in &mut block.stmts {
            self.process_stmt(stmt);
        }
        if let Some(HirStmt::Expr(value) | HirStmt::Return(Some(value))) = block.stmts.last() {
            if let Some(actual) =
                crate::type_services::callable::value_representation(&block.ty, &value.ty)
            {
                if block_returns_agree(block, &actual) {
                    block.ty = actual;
                } else if has_measured_storage(&actual) {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::new(
                            "return paths require an agreed field representation".into(),
                            value.span.clone(),
                        )
                        .with_code(crate::diagnostic::DiagnosticCode::Mono),
                    );
                }
            }
        }
        self.var_types = enclosing_var_types;
    }

    fn process_stmt(&mut self, stmt: &mut HirStmt) {
        match stmt {
            HirStmt::Let {
                local_id,
                ty,
                value,
                ..
            } => {
                self.process_expr(value);
                *ty = value.ty.clone();
                self.monomorphize_drop_for_type(ty, Some(value.span.clone()));
                let ty_id = self.intern_type(ty);
                self.var_types.insert(*local_id, ty_id);
            }
            HirStmt::Expr(expr) => {
                self.process_expr(expr);
                self.monomorphize_drop_for_type(&expr.ty, Some(expr.span.clone()));
            }
            HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
                self.process_expr(expr);
                self.monomorphize_drop_for_type(&expr.ty, Some(expr.span.clone()));
            }
            _ => {}
        }
    }

    // Pattern locals carry the selected value layout, not the inference-era
    // arrow header. This supplies no callable demand or producer certificate.
    fn process_pattern_bindings(&mut self, pattern: &crate::hir::HirPattern, ty: &Type) {
        use crate::hir::HirPattern;

        let instantiate = |owner, args: &[Type], field: &Type| {
            let generics = self.nominal_generic_params.get(&owner)?;
            if generics.len() != args.len() {
                return None;
            }
            let substitution = generics.iter().copied().zip(args.iter().cloned()).collect();
            crate::type_services::substitution::instantiate(field, &substitution).ok()
        };
        match (pattern, ty) {
            (HirPattern::Binding { local_id, .. }, _) => {
                let ty = self.intern_type(ty);
                self.var_types.insert(*local_id, ty);
            }
            (HirPattern::Tuple(patterns), Type::Tuple(fields))
                if patterns.len() == fields.len() =>
            {
                for (pattern, field) in patterns.iter().zip(fields) {
                    self.process_pattern_bindings(pattern, field);
                }
            }
            (
                HirPattern::Enum(_, _, Some(location), patterns),
                Type::Enum { id, args } | Type::EnumRepresentation { id, args, .. },
            ) if location.owner == *id => {
                let fields = if let Type::EnumRepresentation { variants, .. } = ty {
                    variants.get(location.variant_id.0 as usize).cloned()
                } else {
                    self.nominal_variant_fields
                        .get(&(*id, location.variant_id))
                        .and_then(|fields| {
                            fields
                                .iter()
                                .map(|field| instantiate(*id, args, field))
                                .collect::<Option<Vec<_>>>()
                        })
                };
                if let Some(fields) = fields.filter(|fields| fields.len() == patterns.len()) {
                    for (pattern, field) in patterns.iter().zip(fields) {
                        self.process_pattern_bindings(pattern, &field);
                    }
                }
            }
            (
                HirPattern::Struct(_, Some(owner), _, patterns),
                Type::Struct { id, args } | Type::StructRepresentation { id, args, .. },
            ) if owner == id => {
                let fields = patterns
                    .iter()
                    .map(|pattern| {
                        let location = pattern.field.as_ref()?;
                        if location.owner != *id {
                            return None;
                        }
                        let index = self
                            .nominal_field_ids
                            .get(id)?
                            .iter()
                            .position(|field| *field == location.field_id)?;
                        match ty {
                            Type::StructRepresentation { fields, .. } => fields.get(index).cloned(),
                            _ => instantiate(
                                *id,
                                args,
                                self.nominal_field_types.get(id)?.get(index)?,
                            ),
                        }
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(fields) = fields {
                    for (pattern, field) in patterns.iter().zip(fields) {
                        self.process_pattern_bindings(&pattern.pattern, &field);
                    }
                }
            }
            // Borrowed storage and alternatives require independent agreement;
            // their inferred bindings remain conservative.
            _ => {}
        }
    }

    fn process_expr(&mut self, expr: &mut HirExpr) {
        expr.ty = self.substitute_generics(&expr.ty);

        match &mut expr.kind {
            HirExprKind::PackagePack { payload, proof } => {
                self.process_expr(payload);
                proof.visit_types_mut(&mut |ty| *ty = self.substitute_generics(ty));
                let result = self
                    .ensure_payload_schemas(&expr.ty, &mut Default::default())
                    .and_then(|_| {
                        let Type::Exists(package) = &expr.ty else {
                            return Err("package construction target is not existential".into());
                        };
                        let object =
                            crate::type_services::substitution::existential_object_schema(package)
                                .map_err(|error| error.to_string())?;
                        let object = self.ensure_object_schema(&Type::Object(Box::new(object)))?;
                        self.materialize_object_table(
                            object,
                            &proof.witness,
                            &proof.views,
                            &expr.span,
                        )?;
                        Ok(())
                    });
                self.monomorphize_drop_for_type(&payload.ty, Some(payload.span.clone()));
                if let Err(message) = result {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::new(message, expr.span.clone())
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::PackageOpen { .. } => {
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::new(
                        "package opening requires a descriptor-backed scoped body".into(),
                        expr.span.clone(),
                    )
                    .with_code(crate::diagnostic::DiagnosticCode::Mono),
                );
            }
            HirExprKind::Open {
                source, binding, ..
            } => {
                self.process_expr(source);
                binding.visit_types_mut(&mut |ty| *ty = self.substitute_generics(ty));
                // A witness body requires scoped descriptor-backed lowering;
                // ordinary specialization must not invent a concrete layout.
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::new(
                        "scoped opening requires descriptor-backed body lowering, which is not implemented yet".to_string(),
                        expr.span.clone(),
                    ).with_code(crate::diagnostic::DiagnosticCode::Mono),
                );
            }
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                self.process_expr(owner);
                for arg in args {
                    self.process_expr(arg);
                }
                call.visit_types_mut(&mut |ty| *ty = self.substitute_generics(ty));
                if let Err(message) = self.materialize_owned_object_call(owner, call, &expr.span) {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::new(message, expr.span.clone())
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::ObjectCoercion(inner, coercion) => {
                self.process_expr(inner);
                coercion.visit_types_mut(&mut |ty| *ty = self.substitute_generics(ty));
                if let Err(message) = self.materialize_object_coercion(coercion, &expr.span) {
                    self.diagnostics.push(
                        crate::diagnostic::Diagnostic::new(message, expr.span.clone())
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                    );
                }
            }
            HirExprKind::Var(_) => {}
            HirExprKind::ResolvedVar(reference) => {
                if let HirVarTarget::Local(id) = reference.target {
                    if let Some(ty) = self.var_types.get(&id) {
                        expr.ty = self.type_for(*ty);
                    }
                    return;
                }
                if let HirVarTarget::Function(id) = reference.target {
                    self.process_concrete_definition(id);
                    if let Some(definition) = self.concrete_functions.get(&id) {
                        if let Type::Function { ret, .. } = &mut expr.ty {
                            if let Some(actual) =
                                crate::type_services::callable::value_representation(
                                    ret,
                                    &definition.ret_type,
                                )
                            {
                                **ret = actual;
                            }
                        }
                    }
                }

                let generic_target = match reference.target {
                    HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                        self.generic_function_by_id(id)
                    }
                    HirVarTarget::Instance(_) | HirVarTarget::Local(_) => return,
                };

                if let Some((lookup_name, generic_func)) = generic_target {
                    if !self.contains_generic(&expr.ty) {
                        if let Some(type_args) =
                            self.extract_type_args_from_expr_type(&generic_func, &expr.ty)
                        {
                            if let Some((instance_id, specialized_func_type, _)) = self
                                .monomorphize_with_type_args(
                                    &lookup_name,
                                    &generic_func,
                                    &type_args,
                                )
                            {
                                expr.kind = HirExprKind::ResolvedVar(HirVarRef {
                                    name: lookup_name.clone(),
                                    target: HirVarTarget::Instance(instance_id),
                                });
                                expr.ty = specialized_func_type;
                            }
                        }
                    }
                }
            }
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.process_expr(lhs);
                self.process_expr(rhs);
            }
            HirExprKind::UnaryOp(_, inner) => {
                self.process_expr(inner);
            }
            HirExprKind::Call(func, args, target) => {
                // A generic callee's inferred header can abstract over an actual
                // callable environment. Instantiate its definition from the
                // processed arguments, rather than eagerly freezing that header.
                let mut generic_target = match &func.kind {
                    HirExprKind::ResolvedVar(reference) => match reference.target {
                        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                            self.generic_function_by_id(id)
                        }
                        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => None,
                    },
                    _ => None,
                };
                let nominal_target = match &func.kind {
                    HirExprKind::ResolvedVar(reference) => match reference.target {
                        HirVarTarget::Function(id) => self.concrete_function_by_id(id),
                        _ => None,
                    },
                    _ => None,
                };
                let immediate_lambda = matches!(func.kind, HirExprKind::Lambda { .. });
                if generic_target.is_some() || immediate_lambda {
                    func.ty = self.substitute_generics(&func.ty);
                } else {
                    self.process_expr(func);
                }
                for arg in args.iter_mut() {
                    self.process_expr(arg);
                }
                if generic_target.is_none() {
                    // Specialize only a measured local input layout difference;
                    // ordinary nominal calls keep their canonical native link.
                    generic_target = nominal_target
                        .filter(|function| {
                            !function.body.stmts.is_empty()
                                && function.params.len() == args.len()
                                && function.params.iter().zip(args.iter()).any(
                                    |(parameter, argument)| {
                                        !parameter.is_ref
                                            && crate::type_services::visit::type_any(
                                                &argument.ty,
                                                |ty| {
                                                    matches!(
                                                        ty,
                                                        Type::StructRepresentation { .. }
                                                            | Type::EnumRepresentation { .. }
                                                    )
                                                },
                                            )
                                            && crate::type_services::callable::value_representation(
                                                &parameter.ty,
                                                &argument.ty,
                                            )
                                            .is_some_and(|actual| actual != parameter.ty)
                                    },
                                )
                        })
                        .map(|function| (function.name.clone(), function));
                }
                if immediate_lambda {
                    if let HirExprKind::Lambda { params, .. } = &mut func.kind {
                        self.refine_value_parameters(params, args);
                    }
                    self.process_expr(func);
                    if let Type::Function { ret, .. } = &func.ty {
                        expr.ty = *ret.clone();
                    }
                }

                let args_for_mono: Vec<HirExpr> = args.iter().cloned().collect();

                let had_static_authority = matches!(target, Some(HirCallTarget::StaticMethod(_)));
                if let Some(HirCallTarget::StaticMethod(static_target)) = target.clone() {
                    let owner_ty = self.substitute_generics(&static_target.owner_ty);
                    let mut method_target = static_target.method;
                    method_target.for_each_type_mut(|ty| *ty = self.substitute_generics(ty));
                    match self.monomorphize_static_method_call(
                        &owner_ty,
                        &method_target,
                        &args_for_mono,
                        func.span.clone(),
                    ) {
                        Ok((callee, call_target, ret_ty)) => {
                            *func = Box::new(callee);
                            *target = Some(call_target);
                            expr.ty = ret_ty;
                        }
                        Err(error) => {
                            let diagnostic =
                                error.diagnostic_with_context(&self.diagnostic_context());
                            self.diagnostics.push(diagnostic);
                        }
                    }
                }

                let unresolved_static = matches!(target, Some(HirCallTarget::StaticMethod(_)));
                if !had_static_authority {
                    if let Some((lookup_name, generic_func)) = generic_target {
                        if let Some((instance_id, specialized_func_type, specialized_ret_type)) =
                            self.monomorphize_call(
                                &lookup_name,
                                &generic_func,
                                &args_for_mono,
                                Some(&func.ty),
                            )
                        {
                            *func = Box::new(self.instance_callable_expr(
                                lookup_name.clone(),
                                instance_id,
                                specialized_func_type,
                                func.span.clone(),
                            ));
                            expr.ty = specialized_ret_type;
                        }
                    }
                }
                if let Type::Function { ret, .. } = &func.ty {
                    if let Some(actual) =
                        crate::type_services::callable::value_representation(&expr.ty, ret)
                    {
                        expr.ty = actual;
                    }
                }

                if !unresolved_static {
                    if let HirExprKind::ResolvedVar(reference) = &mut func.kind {
                        match reference.target {
                            HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                                if let Some(instance_id) =
                                    self.zero_substitution_callable_instance(id)
                                {
                                    reference.target = HirVarTarget::Instance(instance_id);
                                }
                            }
                            HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
                        }
                    }
                }

                if let HirExprKind::ResolvedVar(reference) = &func.kind {
                    if let HirVarTarget::Instance(instance_id) = reference.target {
                        *target = Some(HirCallTarget::Instance(instance_id));
                    }
                }

                let callee_func = match &func.kind {
                    HirExprKind::Var(_) => None,
                    HirExprKind::ResolvedVar(reference) => match reference.target {
                        HirVarTarget::Instance(instance_id) => {
                            self.lookup_instance_function(instance_id)
                        }
                        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => self
                            .concrete_function_by_id(id)
                            .or_else(|| self.generic_function_by_id(id).map(|(_, func)| func)),
                        HirVarTarget::Local(_) => None,
                    },
                    _ => None,
                };

                if let Some(callee_func) = callee_func {
                    // The selected definition owns its parameter contracts.
                    // A previously inferred reference can still have abstract
                    // default callable slots; do not use those stale slots to
                    // erase the actual argument's kind during MIR adaptation.
                    if let Type::Function {
                        params,
                        ret,
                        safety,
                        ..
                    } = &mut func.ty
                    {
                        *params = callee_func
                            .params
                            .iter()
                            .map(|param| param.ty.clone())
                            .collect();
                        **ret = callee_func.ret_type.clone();
                        *safety =
                            crate::types::FunctionSafety::from_is_unsafe(callee_func.is_unsafe);
                    }
                    for (i, arg) in args.iter_mut().enumerate() {
                        if i < callee_func.params.len() {
                            let param_ty = &callee_func.params[i].ty;
                            adapt_array_slice_callback(arg, param_ty);
                            let generic_target = match &arg.kind {
                                HirExprKind::ResolvedVar(reference) => match reference.target {
                                    HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                                        self.generic_function_by_id(id)
                                    }
                                    HirVarTarget::Instance(_) | HirVarTarget::Local(_) => None,
                                },
                                _ => None,
                            };

                            if let Some((lookup_name, generic_func)) = generic_target {
                                if let Some(type_args) =
                                    self.extract_type_args_from_expr_type(&generic_func, param_ty)
                                {
                                    if let Some((instance_id, spec_func_type, _)) = self
                                        .monomorphize_with_type_args(
                                            &lookup_name,
                                            &generic_func,
                                            &type_args,
                                        )
                                    {
                                        arg.kind = HirExprKind::ResolvedVar(HirVarRef {
                                            name: lookup_name,
                                            target: HirVarTarget::Instance(instance_id),
                                        });
                                        arg.ty = spec_func_type;
                                    }
                                }
                            }
                        }
                    }
                }
                if let HirExprKind::ResolvedVar(reference) = &func.kind {
                    if let HirVarTarget::Instance(instance) = reference.target {
                        if self
                            .instances
                            .record(instance)
                            .is_some_and(|record| record.provided_by_object)
                        {
                            if let Type::Function { params, .. } = &func.ty {
                                for (parameter, argument) in params.iter().zip(args.iter()) {
                                    // A bodyless entry cannot specialize its field-copy
                                    // or cleanup code from a caller's nominal identity.
                                    if crate::type_services::visit::type_any(&argument.ty, |ty| {
                                        matches!(
                                            ty,
                                            Type::StructRepresentation { .. }
                                                | Type::EnumRepresentation { .. }
                                        )
                                    }) && crate::type_services::callable::value_representation(
                                        parameter,
                                        &argument.ty,
                                    )
                                    .map_or(true, |actual| actual != *parameter)
                                    {
                                        self.diagnostics.push(crate::diagnostic::Diagnostic::new(
                                            "native parameter requires its declared storage representation".into(),
                                            argument.span.clone(),
                                        ).with_code(crate::diagnostic::DiagnosticCode::Mono));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            HirExprKind::MethodCall(recv, method_name, args, _self_receiver, target) => {
                let method_name_clone = method_name.clone();

                target.for_each_type_mut(|ty| *ty = self.substitute_generics(ty));
                let selected_target = target.clone();

                let selected_substitution = selected_target
                    .owner_substitution
                    .iter()
                    .chain(selected_target.method_substitution.iter())
                    .map(|binding| (binding.param, self.intern_type(&binding.ty)))
                    .collect::<HashMap<_, _>>();
                if !selected_substitution.is_empty() {
                    **recv = self.substitute_expr(recv, &selected_substitution);
                    for arg in &mut *args {
                        *arg = self.substitute_expr(arg, &selected_substitution);
                    }
                }

                self.process_expr(recv);
                for arg in &mut *args {
                    self.process_expr(arg);
                }
                if matches!(
                    selected_target.target,
                    crate::hir::HirSelectedMethodTarget::TraitMethod {
                        dispatch: crate::hir::HirTraitDispatchKind::Object,
                        ..
                    }
                ) {
                    let object = match &recv.ty {
                        Type::Reference { inner, .. } => inner.as_ref(),
                        ty => ty,
                    };
                    let prepared = self.ensure_object_schema(object).and_then(|object| {
                        let trait_args = selected_target
                            .trait_args()
                            .iter()
                            .map(|ty| self.intern_type(ty))
                            .collect::<Vec<_>>();
                        let slot =
                            self.object_schemas[&object]
                                .erased_slots
                                .iter()
                                .position(|slot| {
                                    Some(slot.trait_id) == selected_target.trait_id()
                                        && Some(slot.member_id) == selected_target.method_id()
                                        && slot.trait_args == trait_args
                                });
                        if let Some(slot) = slot {
                            if let Some(storage) = self.prepare_erased_virtual_call(
                                object,
                                slot as u32,
                                &selected_target,
                                &expr.span,
                            )? {
                                expr.ty = storage;
                            }
                        }
                        Ok(())
                    });
                    if let Err(message) = prepared {
                        self.diagnostics.push(
                            crate::diagnostic::Diagnostic::new(message, expr.span.clone())
                                .with_code(crate::diagnostic::DiagnosticCode::Mono),
                        );
                    }
                    return;
                }

                if let Some(adapter) =
                    self.native_callable_adapter(recv, args, &selected_target, &expr.ty, &expr.span)
                {
                    *expr = adapter;
                    return;
                }

                let processed_recv = recv.as_ref().clone();
                let processed_call_args: Vec<HirExpr> = args.iter().cloned().collect();
                let mut all_args = vec![processed_recv.clone()];
                all_args.extend(processed_call_args.iter().cloned());

                match &selected_target.target {
                    crate::hir::HirSelectedMethodTarget::TraitMethod { .. }
                    | crate::hir::HirSelectedMethodTarget::ImplMethod {
                        selected_trait: Some(_),
                        ..
                    } => {
                        if let Err(error) =
                            self.monomorphize_trait_method_call(&method_name_clone, &all_args, expr)
                        {
                            let diagnostic =
                                error.diagnostic_with_context(&self.diagnostic_context());
                            self.diagnostics.push(diagnostic);
                        }
                    }
                    crate::hir::HirSelectedMethodTarget::ImplMethod { .. } => {
                        if let Err(error) = self.monomorphize_standalone_method_call(
                            &method_name_clone,
                            &all_args,
                            expr,
                        ) {
                            let diagnostic =
                                error.diagnostic_with_context(&self.diagnostic_context());
                            self.diagnostics.push(diagnostic);
                        }
                    }
                }

                if let HirExprKind::Call(_, call_args, _) = &mut expr.kind {
                    if !call_args.is_empty() {
                        call_args[0] = processed_recv;
                    }
                    for (i, arg) in processed_call_args.into_iter().enumerate() {
                        if i + 1 < call_args.len() {
                            call_args[i + 1] = arg;
                        }
                    }
                }
            }
            HirExprKind::Try {
                expr: carrier,
                branch_method,
                branch_target,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                ..
            } => {
                self.process_expr(carrier);
                branch_method.for_each_type_mut(|ty| *ty = self.substitute_generics(ty));
                *output_ty = self.substitute_generics(output_ty);
                *residual_ty = self.substitute_generics(residual_ty);
                *return_ty = self.substitute_generics(return_ty);

                {
                    let target = branch_method.clone();
                    let branch_ret_ty = Type::Enum {
                        id: *control_flow_enum,
                        args: vec![residual_ty.clone(), output_ty.clone()],
                    };
                    let mut branch_call = HirExpr {
                        kind: HirExprKind::MethodCall(
                            Box::new(carrier.as_ref().clone()),
                            "branch".to_string(),
                            Vec::new(),
                            *branch_self_receiver,
                            target,
                        ),
                        ty: branch_ret_ty,
                        span: carrier.span.clone(),
                    };
                    let args = vec![carrier.as_ref().clone()];
                    if let Err(error) =
                        self.monomorphize_trait_method_call("branch", &args, &mut branch_call)
                    {
                        let diagnostic = error.diagnostic_with_context(&self.diagnostic_context());
                        self.diagnostics.push(diagnostic);
                    }
                    if let HirExprKind::Call(_, _, Some(HirCallTarget::Instance(instance_id))) =
                        branch_call.kind
                    {
                        *branch_target = Some(HirCallTarget::Instance(instance_id));
                    }
                }

                if let HirCallTarget::StaticMethod(static_target) = from_residual_target.clone() {
                    let owner_ty = self.substitute_generics(&static_target.owner_ty);
                    let mut method_target = static_target.method;
                    method_target.for_each_type_mut(|ty| *ty = self.substitute_generics(ty));
                    let args = vec![HirExpr {
                        kind: HirExprKind::Unit,
                        ty: residual_ty.clone(),
                        span: carrier.span.clone(),
                    }];
                    match self.monomorphize_static_method_call(
                        &owner_ty,
                        &method_target,
                        &args,
                        carrier.span.clone(),
                    ) {
                        Ok((_, call_target, _)) => *from_residual_target = call_target,
                        Err(error) => {
                            let diagnostic =
                                error.diagnostic_with_context(&self.diagnostic_context());
                            self.diagnostics.push(diagnostic);
                        }
                    }
                }
            }
            HirExprKind::FieldAccess(inner, _, location) => {
                self.process_expr(inner);
                if let (Type::StructRepresentation { id, fields, .. }, Some(location)) =
                    (&inner.ty, location)
                {
                    if location.owner == *id {
                        if let Some(field) = self
                            .nominal_field_ids
                            .get(id)
                            .and_then(|ids| ids.iter().position(|id| *id == location.field_id))
                            .and_then(|index| fields.get(index))
                        {
                            if let Some(actual) =
                                crate::type_services::callable::value_representation(
                                    &expr.ty, field,
                                )
                            {
                                expr.ty = actual;
                            }
                        }
                    }
                }
            }
            HirExprKind::TupleIndex(inner, index) => {
                self.process_expr(inner);
                if let Type::Tuple(fields) = &inner.ty {
                    if let Some(actual) = fields.get(*index as usize).and_then(|field| {
                        crate::type_services::callable::value_representation(&expr.ty, field)
                    }) {
                        expr.ty = actual;
                    }
                }
            }
            HirExprKind::TupleLiteral(elems) => {
                for element in elems.iter_mut() {
                    self.process_expr(element);
                }
                let actual = Type::Tuple(elems.iter().map(|element| element.ty.clone()).collect());
                if let Some(actual) =
                    crate::type_services::callable::value_representation(&expr.ty, &actual)
                {
                    expr.ty = actual;
                }
            }
            HirExprKind::ArrayLiteral(elems) => {
                for e in elems.iter_mut() {
                    self.process_expr(e);
                }
                if let (Type::Array(element, _), Some(first)) = (&mut expr.ty, elems.first()) {
                    if elems.iter().all(|value| value.ty == first.ty) {
                        if let Some(actual) =
                            crate::type_services::callable::value_representation(element, &first.ty)
                        {
                            **element = actual;
                        }
                    } else if elems.iter().any(|value| {
                        crate::type_services::callable::value_representation(element, &value.ty)
                            .is_some_and(|actual| actual != **element)
                    }) {
                        self.diagnostics.push(
                            crate::diagnostic::Diagnostic::new(
                                "array elements require an agreed storage representation".into(),
                                expr.span.clone(),
                            )
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                        );
                    }
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.process_expr(value);
                if let Type::Array(element, _) = &mut expr.ty {
                    if let Some(actual) =
                        crate::type_services::callable::value_representation(element, &value.ty)
                    {
                        **element = actual;
                    }
                }
            }
            HirExprKind::StructLiteral(_, Some(owner), fields) => {
                for field in fields.iter_mut() {
                    self.process_expr(&mut field.value);
                }
                if let Type::Struct { id, args } = &expr.ty {
                    if id == owner {
                        let actual = self.nominal_field_ids.get(id).and_then(|ids| {
                            ids.iter()
                                .map(|id| {
                                    fields
                                        .iter()
                                        .find(|field| {
                                            field.field.as_ref().is_some_and(|location| {
                                                location.owner == *owner && location.field_id == *id
                                            })
                                        })
                                        .map(|field| field.value.ty.clone())
                                })
                                .collect::<Option<Vec<_>>>()
                        });
                        let declared = self.nominal_field_types.get(id).and_then(|fields| {
                            let parameters = self.nominal_generic_params.get(id)?;
                            if parameters.len() != args.len() {
                                return None;
                            }
                            let substitution = parameters
                                .iter()
                                .copied()
                                .zip(args.iter().cloned())
                                .collect();
                            Some(
                                fields
                                    .iter()
                                    .map(|field| field.substitute_generics(&substitution))
                                    .collect::<Vec<_>>(),
                            )
                        });
                        if let (Some(actual), Some(declared)) = (actual, declared) {
                            // Construction measures every field independently; the
                            // nominal declaration supplies shape, never captures.
                            let measured = declared
                                .iter()
                                .zip(&actual)
                                .map(|(declared, actual)| {
                                    crate::type_services::callable::value_representation(
                                        declared, actual,
                                    )
                                })
                                .collect::<Option<Vec<_>>>();
                            if let Some(measured) =
                                measured.filter(|measured| *measured != declared)
                            {
                                expr.ty = Type::StructRepresentation {
                                    id: *id,
                                    args: args.clone(),
                                    fields: measured,
                                };
                            }
                        }
                    }
                }
            }
            HirExprKind::StructLiteral(_, None, fields) => {
                for field in fields {
                    self.process_expr(&mut field.value);
                }
            }
            HirExprKind::EnumVariant(_, _, values, location) => {
                for e in values.iter_mut() {
                    self.process_expr(e);
                }
                if let (Type::Enum { id, args }, Some(location)) = (&expr.ty, location) {
                    if location.owner == *id {
                        let parameters = self.nominal_generic_params.get(id);
                        let declared = parameters
                            .filter(|parameters| parameters.len() == args.len())
                            .and_then(|parameters| {
                                let substitution = parameters
                                    .iter()
                                    .copied()
                                    .zip(args.iter().cloned())
                                    .collect();
                                let count = self
                                    .nominal_variant_fields
                                    .keys()
                                    .filter(|(owner, _)| owner == id)
                                    .count();
                                (0..count)
                                    .map(|index| {
                                        self.nominal_variant_fields
                                            .get(&(*id, crate::ids::VariantId(index as u32)))?
                                            .iter()
                                            .map(|field| {
                                                crate::type_services::substitution::instantiate(
                                                    field,
                                                    &substitution,
                                                )
                                                .ok()
                                            })
                                            .collect::<Option<Vec<_>>>()
                                    })
                                    .collect::<Option<Vec<_>>>()
                            });
                        if let Some(mut variants) = declared {
                            if let Some(fields) = variants.get_mut(location.variant_id.0 as usize) {
                                if fields.len() == values.len() {
                                    let measured = fields
                                        .iter()
                                        .zip(values.iter())
                                        .map(|(field, value)| {
                                            crate::type_services::callable::value_representation(
                                                field, &value.ty,
                                            )
                                        })
                                        .collect::<Option<Vec<_>>>();
                                    if let Some(measured) =
                                        measured.filter(|measured| measured != fields)
                                    {
                                        *fields = measured;
                                        expr.ty = Type::EnumRepresentation {
                                            id: *id,
                                            args: args.clone(),
                                            variants,
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.process_expr(condition);
                self.process_block(then_branch);
                if let Some(else_b) = else_branch {
                    self.process_block(else_b);
                    if then_branch.ty == else_b.ty {
                        if let Some(actual) = crate::type_services::callable::value_representation(
                            &expr.ty,
                            &then_branch.ty,
                        ) {
                            expr.ty = actual;
                        }
                    } else if [&then_branch.ty, &else_b.ty]
                        .iter()
                        .any(|ty| has_measured_storage(ty))
                    {
                        self.diagnostics.push(
                            crate::diagnostic::Diagnostic::new(
                                "conditional values require an agreed field representation".into(),
                                expr.span.clone(),
                            )
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                        );
                    }
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.process_expr(scrutinee);
                for arm in arms.iter_mut() {
                    let enclosing_var_types = self.var_types.clone();
                    // A known pointee supplies storage, not permission to move
                    // it: MIR still makes borrowed pattern bindings references.
                    self.process_pattern_bindings(&arm.pattern, &scrutinee.ty);
                    if let Some(guard) = &mut arm.guard {
                        self.process_expr(guard);
                    }
                    self.process_block(&mut arm.body);
                    self.var_types = enclosing_var_types;
                }
                if let Some(first) = arms.first() {
                    if arms.iter().all(|arm| arm.body.ty == first.body.ty) {
                        if let Some(actual) = crate::type_services::callable::value_representation(
                            &expr.ty,
                            &first.body.ty,
                        ) {
                            expr.ty = actual;
                        }
                    } else if arms.iter().any(|arm| has_measured_storage(&arm.body.ty)) {
                        self.diagnostics.push(
                            crate::diagnostic::Diagnostic::new(
                                "matched values require an agreed field representation".into(),
                                expr.span.clone(),
                            )
                            .with_code(crate::diagnostic::DiagnosticCode::Mono),
                        );
                    }
                }
            }
            HirExprKind::While { condition, body } => {
                self.process_expr(condition);
                self.process_block(body);
            }
            HirExprKind::For { iter, body, .. } => {
                self.process_expr(iter);
                self.process_block(body);
            }
            HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
                self.process_block(body);
                if let Some(actual) =
                    crate::type_services::callable::value_representation(&expr.ty, &body.ty)
                {
                    expr.ty = actual;
                }
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                let old_var_types = std::mem::take(&mut self.var_types);
                self.process_params(self.current_function_owner, params);
                for capture in captures.iter_mut() {
                    capture.ty = self.substitute_generics(&capture.ty);
                    if let Some(actual) = old_var_types.get(&capture.local_id) {
                        if let Some(actual) = crate::type_services::callable::value_representation(
                            &capture.ty,
                            &self.type_for(*actual),
                        ) {
                            capture.ty = actual;
                        }
                    }
                    let ty_id = self.intern_type(&capture.ty);
                    self.var_types.insert(capture.local_id, ty_id);
                }
                self.process_block(body);
                if let Type::Function {
                    params: types,
                    ret,
                    captures: environment,
                    ..
                } = &mut expr.ty
                {
                    *types = params.iter().map(|param| param.ty.clone()).collect();
                    if let Some(actual) =
                        crate::type_services::callable::value_representation(ret, &body.ty)
                    {
                        **ret = actual;
                    }
                    *environment = captures
                        .iter()
                        .map(|capture| {
                            crate::types::FunctionCapture::new(
                                match capture.kind {
                                    crate::hir::HirClosureCaptureKind::Move => {
                                        crate::types::CaptureKind::Move
                                    }
                                    crate::hir::HirClosureCaptureKind::SharedBorrow => {
                                        crate::types::CaptureKind::SharedBorrow
                                    }
                                    crate::hir::HirClosureCaptureKind::MutableBorrow => {
                                        crate::types::CaptureKind::MutableBorrow
                                    }
                                },
                                capture.ty.clone(),
                            )
                        })
                        .collect();
                }
                expr.ty = crate::type_services::callable::resolve_environment_kind(expr.ty.clone());
                self.var_types = old_var_types;
            }
            HirExprKind::Ref(_, inner) => {
                self.process_expr(inner);
                if let Type::Reference { inner: pointee, .. } = &mut expr.ty {
                    if let Some(actual) =
                        crate::type_services::callable::value_representation(pointee, &inner.ty)
                    {
                        **pointee = actual;
                    }
                }
            }
            HirExprKind::Deref(inner) => {
                self.process_expr(inner);
                match &inner.ty {
                    Type::Reference { inner, .. } | Type::Pointer(inner) => {
                        expr.ty = inner.as_ref().clone();
                    }
                    _ => {}
                }
            }
            HirExprKind::Cast(inner, ty) => {
                self.process_expr(inner);
                *ty = self.substitute_generics(ty);
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.process_expr(lhs);
                self.process_expr(rhs);
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.process_expr(arg);
                }
            }
            _ => {}
        }
    }
}

// A tail's environment cannot describe a different early-return environment.
// Nested lambda returns belong to their own callable, not the containing block.
pub(super) fn block_returns_agree(block: &HirBlock, actual: &Type) -> bool {
    fn expression_agrees(expression: &HirExpr, actual: &Type) -> bool {
        if matches!(expression.kind, HirExprKind::Lambda { .. }) {
            return true;
        }
        let mut agrees = true;
        crate::hir::visit::visit_expr_children(expression, &mut |child| {
            agrees &= match child {
                crate::hir::visit::HirChild::Expr(expression) => {
                    expression_agrees(expression, actual)
                }
                crate::hir::visit::HirChild::Block(block) => block_returns_agree(block, actual),
            };
        });
        agrees
    }
    block.stmts.iter().all(|statement| match statement {
        HirStmt::Return(Some(value)) => {
            crate::type_services::callable::value_representation(actual, &value.ty).as_ref()
                == Some(actual)
                && expression_agrees(value, actual)
        }
        HirStmt::Return(None) => *actual == Type::Unit,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) | HirStmt::Break(Some(value)) => {
            expression_agrees(value, actual)
        }
        _ => true,
    })
}

fn adapt_array_slice_callback(arg: &mut HirExpr, expected: &Type) {
    let Type::Function {
        params: expected_params,
        ret: expected_ret,
        safety: expected_safety,
        ..
    } = expected
    else {
        return;
    };
    let Type::Function {
        params: actual_params,
        ret: actual_ret,
        ..
    } = arg.ty.clone()
    else {
        return;
    };
    if expected_params.len() != actual_params.len()
        || !expected_params
            .iter()
            .zip(actual_params.iter())
            .any(|(expected, actual)| {
                matches!(
                    (expected, actual),
                    (
                        Type::Reference { inner: expected_inner, .. },
                        Type::Reference { inner: actual_inner, .. },
                    ) if matches!(expected_inner.as_ref(), Type::Array(_, _))
                        && matches!(actual_inner.as_ref(), Type::Slice(_))
                )
            })
        || !matches!(&arg.kind, HirExprKind::Lambda { captures, .. } if captures.is_empty())
    {
        return;
    }

    let span = arg.span.clone();
    let params = expected_params
        .iter()
        .enumerate()
        .map(|(index, ty)| crate::hir::HirParam {
            name: format!("__callback_arg_{index}"),
            local_id: crate::ids::HirLocalId(index as u32),
            ty: ty.clone(),
            mutable: false,
            is_ref: false,
        })
        .collect::<Vec<_>>();
    let call_args = params
        .iter()
        .zip(actual_params.iter())
        .map(|(param, actual)| {
            let parameter = HirExpr {
                ty: param.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: param.name.clone(),
                    target: HirVarTarget::Local(param.local_id),
                }),
                span: span.clone(),
            };
            let Type::Reference { inner, mutable } = actual else {
                return parameter;
            };
            let Type::Reference {
                inner: parameter_inner,
                ..
            } = &parameter.ty
            else {
                return parameter;
            };
            if !matches!(parameter_inner.as_ref(), Type::Array(_, _))
                || !matches!(inner.as_ref(), Type::Slice(_))
            {
                return parameter;
            }
            HirExpr {
                ty: Type::Reference {
                    mutable: *mutable,
                    inner: inner.clone(),
                },
                kind: HirExprKind::Intrinsic {
                    name: "ArrayRefToSlice".to_string(),
                    args: vec![parameter],
                },
                span: span.clone(),
            }
        })
        .collect::<Vec<_>>();
    let original = std::mem::replace(
        arg,
        HirExpr {
            ty: Type::Unit,
            kind: HirExprKind::Unit,
            span: span.clone(),
        },
    );
    let call = HirExpr {
        ty: *actual_ret,
        kind: HirExprKind::Call(Box::new(original), call_args, None),
        span: span.clone(),
    };
    arg.ty = Type::function_with_safety(
        expected_params.clone(),
        *expected_ret.clone(),
        *expected_safety,
    );
    arg.kind = HirExprKind::Lambda {
        params,
        body: HirBlock {
            ty: call.ty.clone(),
            stmts: vec![HirStmt::Expr(call)],
        },
        captures: Vec::new(),
    };
}

#[cfg(test)]
mod tests {
    use super::super::hir_types::HirImpl;
    use super::*;
    use crate::hir::{HirCallTarget, HirImplOwner, HirMethodCallTarget, HirParam};
    use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
    use crate::mono::{
        InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord, InstanceSymbols,
    };
    use crate::types::ReceiverMode;
    use crate::types::{GenericParamDecl, GenericParamId, Type};
    use std::collections::HashMap;

    fn empty_function(receiver: Option<ReceiverMode>) -> HirFunction {
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "f".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: crate::types::Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: crate::types::Type::Unit,
            },
            is_curried: false,
            is_method: true,
            self_receiver: receiver,
            is_unsafe: false,
        }
    }

    fn function_with_id(id: DefId, receiver: Option<ReceiverMode>) -> HirFunction {
        let mut function = empty_function(receiver);
        function.id = id;
        function
    }

    #[test]
    fn specialized_lambda_kind_uses_its_resolved_environment_only() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};
        let owner = DefId::new(CrateId(0), LocalDefId(900));
        let parameter = GenericParamId { owner, index: 0 };
        let local = HirLocalId(1);
        for (ty, expected) in [
            (Type::I64, CallableKind::Fn),
            (
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
                CallableKind::Fn,
            ),
            (
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::I64),
                },
                CallableKind::FnOnce,
            ),
            (Type::Array(Box::new(Type::I64), 1), CallableKind::FnOnce),
        ] {
            let mut mono = Monomorphizer::new();
            let substitution = HashMap::from([(parameter, mono.intern_type(&ty))]);
            let mut expression = HirExpr {
                kind: HirExprKind::Lambda {
                    params: vec![],
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    captures: vec![crate::hir::HirClosureCapture {
                        name: "value".into(),
                        local_id: local,
                        kind: crate::hir::HirClosureCaptureKind::Move,
                        mutable: false,
                        ty: Type::Generic(parameter),
                    }],
                },
                ty: Type::Function {
                    params: vec![],
                    ret: Box::new(Type::Unit),
                    safety: crate::types::FunctionSafety::Safe,
                    callable_kind: CallableKind::FnOnce,
                    captures: vec![FunctionCapture::new(
                        CaptureKind::Move,
                        Type::Generic(parameter),
                    )]
                    .into(),
                    result_borrowing: Default::default(),
                },
                span: crate::lexer::Span::test(),
            };
            expression = mono.substitute_expr(&expression, &substitution);
            mono.process_expr(&mut expression);
            let Type::Function {
                callable_kind,
                captures,
                result_borrowing,
                ..
            } = &expression.ty
            else {
                unreachable!()
            };
            assert_eq!(*callable_kind, expected, "{ty:?}");
            assert_eq!(captures[0].ty, ty);
            assert_eq!(
                *result_borrowing,
                crate::types::CallableResultBorrowing::Unknown,
                "capture kind refinement cannot manufacture a borrowing certificate"
            );
            // A parameter's protocol remains a declared obligation, not an
            // environment that specialization is allowed to reclassify.
            let mut abstract_callable = HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Function {
                    params: vec![],
                    ret: Box::new(Type::Unit),
                    safety: crate::types::FunctionSafety::Safe,
                    callable_kind: CallableKind::FnOnce,
                    captures: Box::default(),
                    result_borrowing: Default::default(),
                },
                span: crate::lexer::Span::test(),
            };
            mono.process_expr(&mut abstract_callable);
            assert!(matches!(
                abstract_callable.ty,
                Type::Function {
                    callable_kind: CallableKind::FnOnce,
                    ..
                }
            ));
        }
    }

    #[test]
    fn test_lookup_self_receiver_uses_targeted_trait_impl() {
        let mut mono = Monomorphizer::new();
        let trait_id = DefId::new(CrateId(0), LocalDefId(11));
        let method_id = DefId::new(CrateId(0), LocalDefId(12));

        let first_impl = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            owner: HirImplOwner::Named("Foo".to_string()),
            trait_name: Some("TraitA".to_string()),
            trait_id: Some(DefId::new(CrateId(0), LocalDefId(10))),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("other".to_string(), empty_function(None))]),
        };
        let second_impl = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            owner: HirImplOwner::Named("Foo".to_string()),
            trait_name: Some("TraitB".to_string()),
            trait_id: Some(trait_id),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "target".to_string(),
                function_with_id(method_id, Some(ReceiverMode::Mut)),
            )]),
        };

        mono.trait_impls
            .insert(DefId::new(CrateId(0), LocalDefId(10)), vec![first_impl]);
        mono.trait_impls.insert(trait_id, vec![second_impl]);

        assert_eq!(
            mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
                trait_id,
                method_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ))),
            Some(ReceiverMode::Mut)
        );
    }

    #[test]
    fn test_lookup_self_receiver_does_not_pick_first_same_named_trait_method() {
        let mut mono = Monomorphizer::new();
        let first_trait_id = DefId::new(CrateId(0), LocalDefId(10));
        let second_trait_id = DefId::new(CrateId(0), LocalDefId(11));
        let first_method_id = DefId::new(CrateId(0), LocalDefId(30));
        let second_method_id = DefId::new(CrateId(0), LocalDefId(31));

        let first_impl = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(20)),
            owner: HirImplOwner::Named("Foo".to_string()),
            trait_name: Some("TraitA".to_string()),
            trait_id: Some(first_trait_id),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "target".to_string(),
                function_with_id(first_method_id, Some(ReceiverMode::Move)),
            )]),
        };
        let second_impl = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(21)),
            owner: HirImplOwner::Named("Foo".to_string()),
            trait_name: Some("TraitB".to_string()),
            trait_id: Some(second_trait_id),
            type_name: "Foo".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "target".to_string(),
                function_with_id(second_method_id, Some(ReceiverMode::Mut)),
            )]),
        };

        mono.trait_impls.insert(first_trait_id, vec![first_impl]);
        mono.trait_impls.insert(second_trait_id, vec![second_impl]);

        assert_eq!(
            mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
                second_trait_id,
                second_method_id,
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ))),
            Some(ReceiverMode::Mut)
        );
    }

    fn generic_self_method(_type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        HirFunction {
            id: method_id,
            name: "map".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: DefId::new(CrateId(0), LocalDefId(700)),
                    args: vec![Type::Generic(generic)],
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        }
    }

    fn generic_impl_for_process_test(
        type_name: &str,
        impl_id: DefId,
        trait_id: Option<DefId>,
        method_id: DefId,
    ) -> HirImpl {
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(type_name.to_string()),
            type_name: type_name.to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: trait_id.map(|_| "MapTrait".to_string()),
            trait_id,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "map".to_string(),
                generic_self_method(type_name, impl_id, method_id),
            )]),
        }
    }

    fn selected_process_method_target(impl_id: DefId, method_id: DefId) -> HirMethodCallTarget {
        let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
        target.owner_substitution = vec![crate::hir::HirTypeBinding {
            param: GenericParamId {
                owner: impl_id,
                index: 0,
            },
            ty: Type::I64,
        }];
        target
    }

    fn seed_box_receiver_pattern(mono: &mut Monomorphizer, impl_id: DefId) {
        mono.set_impl_receiver_pattern_for_test(
            impl_id,
            crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })],
            }),
        );
    }

    fn generic_static_impl_for_process_test(
        type_name: &str,
        impl_id: DefId,
        method_id: DefId,
    ) -> HirImpl {
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(type_name.to_string()),
            type_name: type_name.to_string(),
            type_generics: vec![GenericParamDecl::type_param(generic, "T")],
            receiver_pattern: vec![Type::Generic(generic)].into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "make".to_string(),
                HirFunction {
                    id: method_id,
                    name: "make".to_string(),
                    generic_params: Vec::new(),
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
                        ty: Type::Generic(generic),
                    },
                    is_curried: false,
                    is_method: false,
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        }
    }

    fn generic_identity_for_process_test(id: DefId, name: &str) -> HirFunction {
        let generic_id = GenericParamId {
            owner: id,
            index: 0,
        };
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(generic_id),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(generic_id),
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Var("value".to_string()),
                    ty: Type::Generic(generic_id),
                    span: crate::lexer::Span::test(),
                }))],
                ty: Type::Generic(generic_id),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn block_representation_refinement_checks_early_returns_and_separate_lambda_scopes() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety};
        let captured = Type::function_with_metadata(
            vec![],
            Type::I64,
            FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 1),
            )],
        );
        let value = |ty| HirExpr {
            kind: HirExprKind::Unit,
            ty,
            span: crate::lexer::Span::test(),
        };
        let mut block = HirBlock {
            stmts: vec![
                HirStmt::Return(Some(value(Type::function(vec![], Type::I64)))),
                HirStmt::Expr(value(captured.clone())),
            ],
            ty: Type::function(vec![], Type::I64),
        };
        assert!(!block_returns_agree(&block, &captured));
        block.stmts[0] = HirStmt::Expr(HirExpr {
            kind: HirExprKind::Lambda {
                params: vec![],
                captures: vec![],
                body: HirBlock {
                    stmts: vec![HirStmt::Return(Some(value(Type::I64)))],
                    ty: Type::I64,
                },
            },
            ty: Type::function(vec![], Type::I64),
            span: crate::lexer::Span::test(),
        });
        assert!(block_returns_agree(&block, &captured));
    }

    #[test]
    fn match_binding_preserves_actual_environment_without_leaking_between_arms() {
        use crate::hir::{HirMatchArmFor, HirPattern};
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};

        let input = HirLocalId(900);
        let binding = HirLocalId(901);
        let inferred = Type::function(vec![], Type::Unit);
        let actual = Type::function_with_metadata(
            vec![],
            Type::Unit,
            crate::types::FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 2),
            )],
        );
        let local = |id, ty: Type| HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "callback".into(),
                target: HirVarTarget::Local(id),
            }),
            ty,
            span: crate::lexer::Span::test(),
        };
        let mut expression = HirExpr {
            ty: Type::Unit,
            kind: HirExprKind::Match {
                scrutinee: Box::new(local(input, Type::Tuple(vec![inferred.clone()]))),
                arms: vec![
                    HirMatchArmFor {
                        pattern: HirPattern::Tuple(vec![HirPattern::Binding {
                            name: "callback".into(),
                            local_id: binding,
                            mutable: false,
                        }]),
                        guard: None,
                        body: HirBlock {
                            ty: inferred.clone(),
                            stmts: vec![HirStmt::Expr(local(binding, inferred.clone()))],
                        },
                    },
                    HirMatchArmFor {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: HirBlock {
                            ty: inferred.clone(),
                            stmts: vec![HirStmt::Expr(local(binding, inferred.clone()))],
                        },
                    },
                ],
            },
            span: crate::lexer::Span::test(),
        };
        let mut mono = Monomorphizer::new();
        let actual_id = mono.intern_type(&Type::Tuple(vec![actual.clone()]));
        mono.var_types.insert(input, actual_id);
        mono.process_expr(&mut expression);
        let HirExprKind::Match { arms, .. } = expression.kind else {
            panic!("match");
        };
        assert_eq!(arms[0].body.ty, actual);
        assert_eq!(arms[1].body.ty, inferred);
        assert!(!mono.var_types.contains_key(&binding));
        assert_eq!(mono.var_types[&input], actual_id);
    }

    #[test]
    fn nominal_pattern_bindings_use_canonical_payload_layouts_only() {
        use crate::hir::{HirFieldLocation, HirPattern, HirStructPatternField, HirVariantLocation};
        use crate::ids::{FieldId, VariantId};
        use crate::types::{CallableKind, CaptureKind, FunctionCapture, GenericParamId};

        let owner = DefId::new(CrateId(0), LocalDefId(995));
        let other = DefId::new(CrateId(0), LocalDefId(996));
        let generic = GenericParamId { owner, index: 0 };
        let binding = HirLocalId(900);
        let variant = VariantId(2);
        let actual = Type::function_with_metadata(
            vec![],
            Type::Unit,
            crate::types::FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 2),
            )],
        );
        let payload = HirPattern::Binding {
            name: "callback".into(),
            local_id: binding,
            mutable: false,
        };
        let pattern = HirPattern::Enum(
            "ignored spelling".into(),
            "ignored variant spelling".into(),
            Some(HirVariantLocation {
                owner,
                variant_id: variant,
                name: "ignored".into(),
            }),
            vec![payload.clone()],
        );
        let mut mono = Monomorphizer::new();
        mono.nominal_generic_params.insert(owner, vec![generic]);
        mono.nominal_variant_fields
            .insert((owner, variant), vec![Type::Generic(generic)]);
        mono.process_pattern_bindings(
            &pattern,
            &Type::Enum {
                id: owner,
                args: vec![actual.clone()],
            },
        );
        assert_eq!(mono.type_for(mono.var_types[&binding]), actual);
        mono.var_types.clear();
        mono.process_pattern_bindings(
            &pattern,
            &Type::Enum {
                id: other,
                args: vec![actual.clone()],
            },
        );
        assert!(!mono.var_types.contains_key(&binding));
        mono.process_pattern_bindings(
            &pattern,
            &Type::Reference {
                mutable: false,
                inner: Box::new(Type::Enum {
                    id: owner,
                    args: vec![actual.clone()],
                }),
            },
        );
        assert!(!mono.var_types.contains_key(&binding));
        mono.nominal_field_ids.insert(owner, vec![FieldId(3)]);
        mono.nominal_field_types
            .insert(owner, vec![Type::Generic(generic)]);
        let pattern = HirPattern::Struct(
            "ignored".into(),
            Some(owner),
            vec![],
            vec![HirStructPatternField {
                name: "ignored".into(),
                field: Some(HirFieldLocation {
                    owner,
                    field_id: FieldId(3),
                    name: "ignored".into(),
                }),
                pattern: payload,
            }],
        );
        mono.process_pattern_bindings(
            &pattern,
            &Type::Struct {
                id: owner,
                args: vec![actual.clone()],
            },
        );
        assert_eq!(mono.type_for(mono.var_types[&binding]), actual);
    }

    #[test]
    fn immediate_lambda_preserves_actual_callable_environment_through_a_generic_return() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};

        let id = DefId::new(CrateId(0), LocalDefId(990));
        let input = HirLocalId(10);
        let argument = HirLocalId(11);
        let abstract_type = Type::function(vec![], Type::Unit);
        let actual = Type::function_with_metadata(
            vec![],
            Type::Unit,
            crate::types::FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 1),
            )],
        );
        let local = |id, ty: Type| HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "value".into(),
                target: HirVarTarget::Local(id),
            }),
            ty,
            span: crate::lexer::Span::test(),
        };
        let body = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "identity".into(),
                        target: HirVarTarget::Function(id),
                    }),
                    ty: Type::function(vec![abstract_type.clone()], abstract_type.clone()),
                    span: crate::lexer::Span::test(),
                }),
                vec![local(input, abstract_type.clone())],
                Some(HirCallTarget::Function(id)),
            ),
            ty: abstract_type.clone(),
            span: crate::lexer::Span::test(),
        };
        let mut expression = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Lambda {
                        params: vec![HirParam {
                            name: "value".into(),
                            local_id: input,
                            ty: abstract_type.clone(),
                            mutable: false,
                            is_ref: false,
                        }],
                        body: HirBlock {
                            stmts: vec![HirStmt::Expr(body)],
                            ty: abstract_type.clone(),
                        },
                        captures: vec![],
                    },
                    ty: Type::function(vec![abstract_type.clone()], abstract_type.clone()),
                    span: crate::lexer::Span::test(),
                }),
                vec![local(argument, abstract_type.clone())],
                None,
            ),
            ty: abstract_type,
            span: crate::lexer::Span::test(),
        };
        let mut mono = Monomorphizer::new();
        mono.register_generic_function(
            "identity".into(),
            generic_identity_for_process_test(id, "identity"),
        );
        let actual_id = mono.intern_type(&actual);
        mono.var_types.insert(argument, actual_id);
        mono.process_expr(&mut expression);
        assert_eq!(expression.ty, actual);
        let HirExprKind::Call(callee, _, _) = &expression.kind else {
            panic!("immediate call")
        };
        let HirExprKind::Lambda { params, body, .. } = &callee.kind else {
            panic!("lambda retained, not flattened")
        };
        assert_eq!(params[0].ty, actual);
        assert_eq!(body.ty, actual);
        assert_eq!(
            callee.ty,
            Type::function(vec![actual.clone()], actual.clone())
        );
        assert!(!crate::type_services::facts::TypeFacts::is_copy(&actual));
        assert!(mono.diagnostics.0.is_empty());
    }

    #[test]
    fn direct_generic_call_specializes_from_actual_environment_not_stale_callable_header() {
        use crate::types::{CallableKind, CaptureKind, FunctionCapture};

        let id = DefId::new(CrateId(0), LocalDefId(989));
        let input = HirLocalId(10);
        let abstract_callable = Type::function(vec![], Type::Unit);
        let actual = Type::function_with_metadata(
            vec![],
            Type::Unit,
            crate::types::FunctionSafety::Safe,
            CallableKind::FnOnce,
            vec![FunctionCapture::new(
                CaptureKind::Move,
                Type::Array(Box::new(Type::I64), 1),
            )],
        );
        for nested in [false, true] {
            let wrap = |ty: Type| if nested { Type::Tuple(vec![ty]) } else { ty };
            let expected = wrap(abstract_callable.clone());
            let supplied = wrap(actual.clone());
            let mut mono = Monomorphizer::new();
            mono.register_generic_function(
                "identity".into(),
                generic_identity_for_process_test(id, "identity"),
            );
            let actual_id = mono.intern_type(&supplied);
            mono.var_types.insert(input, actual_id);
            let mut expression = HirExpr {
                kind: HirExprKind::Call(
                    Box::new(HirExpr {
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: "identity".into(),
                            target: HirVarTarget::Function(id),
                        }),
                        ty: Type::function(vec![expected.clone()], expected.clone()),
                        span: crate::lexer::Span::test(),
                    }),
                    vec![HirExpr {
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: "producer".into(),
                            target: HirVarTarget::Local(input),
                        }),
                        ty: expected.clone(),
                        span: crate::lexer::Span::test(),
                    }],
                    Some(HirCallTarget::Function(id)),
                ),
                ty: expected,
                span: crate::lexer::Span::test(),
            };
            mono.process_expr(&mut expression);
            assert_eq!(
                expression.ty, supplied,
                "the generic value includes its actual environment"
            );
            let HirExprKind::Call(_, _, Some(HirCallTarget::Instance(instance))) = expression.kind
            else {
                panic!("generic call must resolve by definition identity");
            };
            let record = mono.instances.record(instance).unwrap();
            assert_eq!(record.substitution, vec![actual_id]);
            let body = mono.instances.pre_mir_body(instance).unwrap();
            assert_eq!(body.params[0].ty, supplied);
            assert_eq!(body.ret_type, supplied);
            assert!(mono.diagnostics.0.is_empty());
        }
    }

    fn callback_acceptor_for_process_test(id: DefId, name: &str, callback_ty: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "callback".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: callback_ty,
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn process_targetless_generic_function_call_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let id = DefId::new(CrateId(0), LocalDefId(910));
        let generic = HirFunction {
            id,
            name: "identity".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: 0,
                },
                "T",
            )],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(GenericParamId {
                    owner: id,
                    index: 0,
                }),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Var("value".to_string()),
                    ty: Type::Generic(GenericParamId {
                        owner: id,
                        index: 0,
                    }),
                    span: crate::lexer::Span::test(),
                }))],
                ty: Type::Generic(GenericParamId {
                    owner: id,
                    index: 0,
                }),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        mono.register_generic_function("identity".to_string(), generic);
        mono.resolver.item_paths.insert("identity".to_string(), id);
        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("identity".to_string()),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(21),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                None,
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        assert!(matches!(&callee.kind, HirExprKind::Var(name) if name == "identity"));
        assert_eq!(target, &None);
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_targetless_generic_function_value_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let id = DefId::new(CrateId(0), LocalDefId(911));
        let generic = generic_identity_for_process_test(id, "identity");
        mono.register_generic_function("identity".to_string(), generic);
        mono.resolver.item_paths.insert("identity".to_string(), id);

        let mut expr = HirExpr {
            kind: HirExprKind::Var("identity".to_string()),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "identity"));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_resolved_generic_function_value_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        let identity_id = DefId::new(CrateId(0), LocalDefId(970));
        let wrong_id = DefId::new(CrateId(0), LocalDefId(971));
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_generic_function("identity".to_string(), identity.clone());
        mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "wrong_alias".to_string(),
                target: HirVarTarget::Function(identity_id),
            }),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::ResolvedVar(reference) = &expr.kind else {
            panic!("function value should remain a resolved var");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("resolved generic function value should target an instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_resolved_generic_call_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        let identity_id = DefId::new(CrateId(0), LocalDefId(972));
        let wrong_id = DefId::new(CrateId(0), LocalDefId(973));
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_generic_function("identity".to_string(), identity.clone());
        mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Function(identity_id),
                    }),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(identity_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("callee should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("callee should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_external_generic_function_value_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        mono.register_test_dependency_identity(CrateId(1));
        let identity_id = DefId::new(CrateId(1), LocalDefId(984));
        let wrong_id = DefId::new(CrateId(1), LocalDefId(985));
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_external_generic_function("dep::identity".to_string(), identity);
        mono.register_external_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("dep::identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "wrong_alias".to_string(),
                target: HirVarTarget::Extern(identity_id),
            }),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::ResolvedVar(reference) = &expr.kind else {
            panic!("external function value should remain a resolved var");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("external generic function value should target an instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_external_generic_call_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        mono.register_test_dependency_identity(CrateId(1));
        let identity_id = DefId::new(CrateId(1), LocalDefId(986));
        let wrong_id = DefId::new(CrateId(1), LocalDefId(987));
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_external_generic_function("dep::identity".to_string(), identity);
        mono.register_external_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("dep::identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Extern(identity_id),
                    }),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(identity_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("callee should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("callee should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_object_backed_concrete_function_call_uses_registered_instance() {
        let mut mono = Monomorphizer::new();
        let answer_id = DefId::new(CrateId(1), LocalDefId(994));
        let answer = callback_acceptor_for_process_test(answer_id, "answer", Type::Unit);
        let key = InstanceKey::new(InstanceOrigin::Function(answer_id), Vec::new());
        let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
            id,
            origin: key.origin.clone(),
            substitution: key.substitution.clone(),
            symbols: InstanceSymbols::new("dep::answer", "dep::answer"),
            declared: Some(answer.clone()),
            provided_by_object: true,
            is_specialization: false,
        });
        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "answer".to_string(),
                        target: HirVarTarget::Function(answer_id),
                    }),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(answer_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("callee should remain resolved");
        };
        assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
        assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    }

    #[test]
    fn process_object_backed_static_impl_method_call_uses_registered_instance() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(1), LocalDefId(995));
        let method_id = DefId::new(CrateId(1), LocalDefId(996));
        let method = callback_acceptor_for_process_test(method_id, "dealloc", Type::Unit);
        let key = InstanceKey::new(
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            },
            Vec::new(),
        );
        let instance_id = mono.instances.intern(key.clone(), |id| InstanceRecord {
            id,
            origin: key.origin.clone(),
            substitution: key.substitution.clone(),
            symbols: InstanceSymbols::new("__rock_Global_dealloc", "__rock_Global_dealloc"),
            declared: Some(method.clone()),
            provided_by_object: true,
            is_specialization: false,
        });
        mono.generic_impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Global".to_string()),
                type_name: "Global".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("dealloc".to_string(), method)]),
            },
        );
        mono.set_impl_receiver_pattern_for_test(
            impl_id,
            crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
        );

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "dealloc".to_string(),
                        target: HirVarTarget::Function(method_id),
                    }),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::StaticMethod(
                    crate::hir::HirStaticMethodTarget {
                        owner_ty: Type::Unit,
                        method: HirMethodCallTarget::impl_method(impl_id, method_id, None),
                    },
                )),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("callee should remain resolved");
        };
        assert_eq!(reference.target, HirVarTarget::Instance(instance_id));
        assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    }

    #[test]
    fn generic_function_by_id_requires_def_id_name_metadata() {
        let mut mono = Monomorphizer::new();
        let id = DefId::new(CrateId(0), LocalDefId(977));
        let function = generic_identity_for_process_test(id, "identity");

        mono.generic_functions.insert(id, function);

        assert!(mono.generic_function_by_id(id).is_none());
    }

    #[test]
    fn generic_function_payloads_with_matching_display_names_remain_distinct_by_def_id() {
        let mut mono = Monomorphizer::new();
        let first_id = DefId::new(CrateId(0), LocalDefId(978));
        let second_id = DefId::new(CrateId(1), LocalDefId(978));

        mono.register_generic_function(
            "shared".to_string(),
            generic_identity_for_process_test(first_id, "stale-payload-name"),
        );
        mono.register_generic_function(
            "shared".to_string(),
            generic_identity_for_process_test(second_id, "stale-payload-name"),
        );
        assert_eq!(mono.generic_functions.len(), 2);
        assert_eq!(
            mono.generic_function_by_id(second_id)
                .expect("selected DefId should retain its payload")
                .1
                .id,
            second_id
        );
    }

    #[test]
    fn local_type_tracking_uses_hir_local_id_not_source_name() {
        let mut mono = Monomorphizer::new();
        let mut function = empty_function(None);
        function.body = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "value".to_string(),
                    local_id: HirLocalId(0),
                    ty: Type::I64,
                    value: HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Let {
                    name: "value".to_string(),
                    local_id: HirLocalId(1),
                    ty: Type::Bool,
                    value: HirExpr {
                        kind: HirExprKind::BoolLiteral(true),
                        ty: Type::Bool,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Expr(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "value".to_string(),
                        target: HirVarTarget::Local(HirLocalId(0)),
                    }),
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::Unit,
        };

        let processed = mono.process_function(function);
        let HirStmt::Expr(expr) = &processed.body.stmts[2] else {
            panic!("expected expression statement");
        };

        assert_eq!(expr.ty, Type::I64);
    }

    #[test]
    fn parameter_type_tracking_uses_hir_local_id_not_source_name() {
        let mut mono = Monomorphizer::new();
        let mut function = empty_function(None);
        function.params = vec![HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }];
        function.ret_type = Type::I64;
        function.body = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "value".to_string(),
                    local_id: HirLocalId(1),
                    ty: Type::Bool,
                    value: HirExpr {
                        kind: HirExprKind::BoolLiteral(true),
                        ty: Type::Bool,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Expr(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "value".to_string(),
                        target: HirVarTarget::Local(HirLocalId(0)),
                    }),
                    ty: Type::Bool,
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::I64,
        };

        let processed = mono.process_function(function);
        let HirStmt::Expr(expr) = &processed.body.stmts[1] else {
            panic!("expected expression statement");
        };

        assert_eq!(expr.ty, Type::I64);
    }

    #[test]
    fn function_local_type_frame_restores_the_enclosing_function() {
        let mut mono = Monomorphizer::new();
        let outer_ty = mono.intern_type(&Type::Tuple(vec![Type::I64, Type::Bool]));
        mono.var_types.insert(HirLocalId(0), outer_ty);
        let outer_owner = DefId::new(CrateId(0), LocalDefId(981));
        mono.current_function_owner = Some(outer_owner);
        let mut function = empty_function(None);
        function.params = vec![HirParam {
            name: "argument".into(),
            local_id: HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }];
        function.ret_type = Type::I64;
        function.body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "argument".into(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        };
        let processed = mono.process_function(function);
        let HirStmt::Expr(argument) = &processed.body.stmts[0] else {
            panic!("expected argument expression");
        };
        assert_eq!(argument.ty, Type::I64);
        assert_eq!(mono.var_types, HashMap::from([(HirLocalId(0), outer_ty)]));
        assert_eq!(mono.current_function_owner, Some(outer_owner));
    }

    #[test]
    fn nested_block_type_frame_keeps_enclosing_bindings_without_exporting_inner_bindings() {
        let mut mono = Monomorphizer::new();
        let outer_ty = mono.intern_type(&Type::I64);
        mono.var_types.insert(HirLocalId(0), outer_ty);
        let mut block = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "inner".into(),
                    local_id: HirLocalId(1),
                    ty: Type::Bool,
                    value: HirExpr {
                        kind: HirExprKind::BoolLiteral(true),
                        ty: Type::Bool,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Expr(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "outer".into(),
                        target: HirVarTarget::Local(HirLocalId(0)),
                    }),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::I64,
        };
        mono.process_block(&mut block);
        let HirStmt::Expr(outer) = &block.stmts[1] else {
            panic!("expected outer binding expression");
        };
        assert_eq!(outer.ty, Type::I64);
        assert_eq!(mono.var_types, HashMap::from([(HirLocalId(0), outer_ty)]));
    }

    #[test]
    fn process_resolved_callee_param_lookup_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        let apply_id = DefId::new(CrateId(0), LocalDefId(978));
        let identity_id = DefId::new(CrateId(0), LocalDefId(979));
        let wrong_id = DefId::new(CrateId(0), LocalDefId(980));
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let wrong = callback_acceptor_for_process_test(
            wrong_id,
            "wrong_alias",
            Type::function(vec![Type::I32], Type::I32),
        );

        mono.register_concrete_function("apply".to_string(), apply);
        mono.register_generic_function("identity".to_string(), identity);
        mono.register_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("apply".to_string(), apply_id);
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Function(apply_id),
                    }),
                    ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::Var("identity".to_string()),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(apply_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        assert_eq!(args[0].ty, Type::function(vec![Type::I64], Type::I64));
    }

    #[test]
    fn process_resolved_generic_function_argument_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        let apply_id = DefId::new(CrateId(0), LocalDefId(981));
        let identity_id = DefId::new(CrateId(0), LocalDefId(982));
        let wrong_id = DefId::new(CrateId(0), LocalDefId(983));
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_concrete_function("apply".to_string(), apply);
        mono.register_generic_function("identity".to_string(), identity);
        mono.register_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("apply".to_string(), apply_id);
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "apply".to_string(),
                        target: HirVarTarget::Function(apply_id),
                    }),
                    ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Function(identity_id),
                    }),
                    ty: Type::function(
                        vec![Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        })],
                        Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        }),
                    ),
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(apply_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
            panic!("function argument should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("function argument should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
        assert_eq!(args[0].ty, Type::function(vec![Type::I64], Type::I64));
    }

    #[test]
    fn process_targetless_generic_function_argument_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let apply_id = DefId::new(CrateId(0), LocalDefId(991));
        let identity_id = DefId::new(CrateId(0), LocalDefId(992));
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let identity = generic_identity_for_process_test(identity_id, "identity");

        mono.register_concrete_function("apply".to_string(), apply);
        mono.register_generic_function("identity".to_string(), identity);
        mono.resolver
            .item_paths
            .insert("apply".to_string(), apply_id);
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "apply".to_string(),
                        target: HirVarTarget::Function(apply_id),
                    }),
                    ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::Var("identity".to_string()),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(apply_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        assert!(matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_targetless_concrete_callee_does_not_specialize_argument_by_name() {
        let mut mono = Monomorphizer::new();
        let apply_id = DefId::new(CrateId(0), LocalDefId(1988));
        let identity_id = DefId::new(CrateId(0), LocalDefId(1989));
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let identity = generic_identity_for_process_test(identity_id, "identity");

        mono.register_concrete_function("apply".to_string(), apply);
        mono.register_generic_function("identity".to_string(), identity);
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("apply".to_string()),
                    ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "identity".to_string(),
                        target: HirVarTarget::Function(identity_id),
                    }),
                    ty: Type::function(
                        vec![Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        })],
                        Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        }),
                    ),
                    span: crate::lexer::Span::test(),
                }],
                None,
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        assert!(matches!(args[0].kind, HirExprKind::ResolvedVar(_)));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_external_generic_function_argument_uses_def_id_not_reference_name() {
        let mut mono = Monomorphizer::new();
        mono.register_test_dependency_identity(CrateId(1));
        let apply_id = DefId::new(CrateId(0), LocalDefId(988));
        let identity_id = DefId::new(CrateId(1), LocalDefId(989));
        let wrong_id = DefId::new(CrateId(1), LocalDefId(990));
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_concrete_function("apply".to_string(), apply);
        mono.register_external_generic_function("dep::identity".to_string(), identity);
        mono.register_external_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("apply".to_string(), apply_id);
        mono.resolver
            .item_paths
            .insert("dep::identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "apply".to_string(),
                        target: HirVarTarget::Function(apply_id),
                    }),
                    ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Extern(identity_id),
                    }),
                    ty: Type::function(
                        vec![Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        })],
                        Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        }),
                    ),
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(apply_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
            panic!("function argument should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("function argument should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_external_callee_param_lookup_uses_def_id() {
        let mut mono = Monomorphizer::new();
        mono.register_test_dependency_identity(CrateId(1));
        let apply_id = DefId::new(CrateId(1), LocalDefId(991));
        let identity_id = DefId::new(CrateId(1), LocalDefId(992));
        let wrong_id = DefId::new(CrateId(1), LocalDefId(993));
        let apply_generic_id = GenericParamId {
            owner: apply_id,
            index: 0,
        };
        let apply = callback_acceptor_for_process_test(
            apply_id,
            "apply",
            Type::function(vec![Type::I64], Type::I64),
        );
        let identity = generic_identity_for_process_test(identity_id, "identity");
        let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

        mono.register_external_generic_function("dep::apply".to_string(), apply);
        mono.register_external_generic_function("dep::identity".to_string(), identity);
        mono.register_external_generic_function("wrong_alias".to_string(), wrong);
        mono.resolver
            .item_paths
            .insert("dep::apply".to_string(), apply_id);
        mono.resolver
            .item_paths
            .insert("dep::identity".to_string(), identity_id);
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), wrong_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "dep::apply".to_string(),
                        target: HirVarTarget::Extern(apply_id),
                    }),
                    ty: Type::function(
                        vec![Type::function(vec![Type::I64], Type::I64)],
                        Type::Generic(apply_generic_id),
                    ),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Extern(identity_id),
                    }),
                    ty: Type::function(
                        vec![Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        })],
                        Type::Generic(GenericParamId {
                            owner: identity_id,
                            index: 0,
                        }),
                    ),
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(apply_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
            panic!("function argument should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("function argument should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
        assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    }

    #[test]
    fn process_instance_callee_does_not_fall_back_to_static_impl_name_lookup() {
        let mut mono = Monomorphizer::new();
        let free_id = DefId::new(CrateId(0), LocalDefId(974));
        let impl_id = DefId::new(CrateId(0), LocalDefId(975));
        let method_id = DefId::new(CrateId(0), LocalDefId(976));
        let free_function = generic_identity_for_process_test(free_id, "Box_make");

        mono.register_generic_function("Box_make".to_string(), free_function);
        mono.generic_impls.insert(
            impl_id,
            generic_static_impl_for_process_test("Box", impl_id, method_id),
        );
        mono.resolver
            .item_paths
            .insert("Box_make".to_string(), free_id);

        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "Box_make".to_string(),
                        target: HirVarTarget::Function(free_id),
                    }),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(42),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(free_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("expression should remain a call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("callee should be instance-backed");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("callee should target a monomorphized instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
        assert_eq!(record.origin, InstanceOrigin::Function(free_id));
        assert_ne!(
            record.origin,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            }
        );
    }

    #[test]
    fn process_static_impl_function_target_is_not_materialized_without_authority() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(950));
        let method_id = DefId::new(CrateId(0), LocalDefId(951));
        let colliding_function_id = DefId::new(CrateId(0), LocalDefId(952));
        let generic_id = GenericParamId {
            owner: colliding_function_id,
            index: 0,
        };
        mono.generic_impls.insert(
            impl_id,
            generic_static_impl_for_process_test("Box", impl_id, method_id),
        );
        mono.generic_functions.insert(
            colliding_function_id,
            HirFunction {
                id: colliding_function_id,
                name: "wrong_alias".to_string(),
                generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
                generic_bounds: HashMap::new().into(),
                params: vec![HirParam {
                    name: "value".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(generic_id),
                    mutable: false,
                    is_ref: false,
                }],
                ret_type: Type::Generic(generic_id),
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::Generic(generic_id),
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        mono.resolver
            .item_paths
            .insert("wrong_alias".to_string(), colliding_function_id);
        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "wrong_alias".to_string(),
                        target: HirVarTarget::Function(method_id),
                    }),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(7),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }],
                Some(HirCallTarget::Function(method_id)),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, target) = &expr.kind else {
            panic!("static impl call should remain a call");
        };
        assert!(matches!(
            &callee.kind,
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Function(id),
                ..
            }) if *id == method_id
        ));
        assert_eq!(target, &Some(HirCallTarget::Function(method_id)));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_static_impl_function_value_is_not_materialized_without_authority() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(960));
        let method_id = DefId::new(CrateId(0), LocalDefId(961));
        mono.generic_impls.insert(
            impl_id,
            generic_static_impl_for_process_test("Box", impl_id, method_id),
        );
        let mut expr = HirExpr {
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "Box_make".to_string(),
                target: HirVarTarget::Function(method_id),
            }),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::ResolvedVar(reference) = &expr.kind else {
            panic!("static impl function value should remain a resolved var");
        };
        assert_eq!(reference.target, HirVarTarget::Function(method_id));
        assert!(mono.instances.records().next().is_none());
        assert_eq!(expr.ty, Type::function(vec![Type::I64], Type::I64));
    }

    #[test]
    fn process_call_does_not_resolve_callee_by_backend_symbol() {
        let mut mono = Monomorphizer::new();
        let apply_id = DefId::new(CrateId(0), LocalDefId(915));
        let identity_id = DefId::new(CrateId(0), LocalDefId(916));
        let generic_id = GenericParamId {
            owner: identity_id,
            index: 0,
        };
        let generic_identity = HirFunction {
            id: identity_id,
            name: "identity".to_string(),
            generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Generic(generic_id),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(generic_id),
            body: HirBlock {
                stmts: vec![HirStmt::Return(Some(HirExpr {
                    kind: HirExprKind::Var("value".to_string()),
                    ty: Type::Generic(generic_id),
                    span: crate::lexer::Span::test(),
                }))],
                ty: Type::Generic(generic_id),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        mono.register_generic_function("identity".to_string(), generic_identity);
        mono.resolver
            .item_paths
            .insert("identity".to_string(), identity_id);

        let apply_substitution = mono.intern_types(&[Type::I64]);
        let apply_key =
            crate::mono::InstanceKey::new(InstanceOrigin::Function(apply_id), apply_substitution);
        let apply_body = HirFunction {
            id: apply_id,
            name: "apply".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "callback".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::function(vec![Type::I64], Type::I64),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let apply_instance =
            mono.instances
                .intern(apply_key.clone(), |id| crate::mono::InstanceRecord {
                    id,
                    origin: apply_key.origin.clone(),
                    substitution: apply_key.substitution.clone(),
                    symbols: crate::mono::InstanceSymbols::new("apply", "apply_mono_0"),
                    declared: None,
                    provided_by_object: false,
                    is_specialization: true,
                });
        mono.instances
            .insert_pre_mir_body(apply_instance, apply_body);
        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("apply_mono_0".to_string()),
                    ty: Type::function(vec![Type::I64], Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::Var("identity".to_string()),
                    ty: Type::function(vec![Type::Generic(generic_id)], Type::Generic(generic_id)),
                    span: crate::lexer::Span::test(),
                }],
                None,
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(_, args, _) = &expr.kind else {
            panic!("expression should remain a call");
        };
        assert_eq!(mono.instances.len(), 1);
        assert!(
            matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"),
            "backend symbols must not recover instance bodies for argument specialization"
        );
    }

    #[test]
    fn process_selected_inherent_method_skips_trait_specialization_path() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(701));
        let inherent_method_id = DefId::new(CrateId(0), LocalDefId(702));
        let trait_method_id = DefId::new(CrateId(0), LocalDefId(703));
        let trait_id = DefId::new(CrateId(0), LocalDefId(704));
        mono.resolver
            .item_names_by_id
            .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
        mono.generic_impls.insert(
            impl_id,
            generic_impl_for_process_test("Box", impl_id, None, inherent_method_id),
        );
        mono.trait_impls.insert(
            trait_id,
            vec![generic_impl_for_process_test(
                "Box",
                impl_id,
                Some(trait_id),
                trait_method_id,
            )],
        );
        seed_box_receiver_pattern(&mut mono, impl_id);
        let recv = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::I64],
            },
            span: crate::lexer::Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv),
                "map".to_string(),
                Vec::new(),
                Some(ReceiverMode::Move),
                selected_process_method_target(impl_id, inherent_method_id),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let origins = mono
            .instances
            .records()
            .map(|record| record.origin.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            origins,
            vec![InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: inherent_method_id,
            }]
        );
    }

    #[test]
    fn process_selected_generic_method_call_uses_instance_target() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(921));
        let method_id = DefId::new(CrateId(0), LocalDefId(922));
        mono.resolver
            .item_names_by_id
            .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
        mono.generic_impls.insert(
            impl_id,
            generic_impl_for_process_test("Box", impl_id, None, method_id),
        );
        seed_box_receiver_pattern(&mut mono, impl_id);
        let recv = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::I64],
            },
            span: crate::lexer::Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv),
                "map".to_string(),
                Vec::new(),
                Some(ReceiverMode::Move),
                selected_process_method_target(impl_id, method_id),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        mono.process_expr(&mut expr);

        let HirExprKind::Call(callee, _, _) = &expr.kind else {
            panic!("method call should lower to a direct call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("method callee should be an instance-backed resolved var");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("method callee should target an instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(
            record.origin,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: method_id,
            }
        );
    }

    #[test]
    fn process_targetless_generic_method_call_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let source_span = crate::lexer::Span {
            file_path: "/test.rk".into(),
            start: 0,
            end: 3,
        };
        let impl_id = DefId::new(CrateId(0), LocalDefId(941));
        let method_id = DefId::new(CrateId(0), LocalDefId(942));
        mono.resolver
            .item_names_by_id
            .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
        mono.generic_impls.insert(
            impl_id,
            generic_impl_for_process_test("Box", impl_id, None, method_id),
        );
        seed_box_receiver_pattern(&mut mono, impl_id);
        let recv = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::I64],
            },
            span: source_span.clone(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv),
                "map".to_string(),
                Vec::new(),
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    DefId::new(CrateId(0), LocalDefId(999)),
                    DefId::new(CrateId(0), LocalDefId(998)),
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I64,
            span: source_span,
        };

        mono.process_expr(&mut expr);

        assert!(
            matches!(expr.kind, HirExprKind::MethodCall(..)),
            "targetless semantic method dispatch must not be rediscovered by name"
        );
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn process_reused_generic_method_call_keeps_instance_target() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(931));
        let method_id = DefId::new(CrateId(0), LocalDefId(932));
        mono.resolver
            .item_names_by_id
            .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
        mono.generic_impls.insert(
            impl_id,
            generic_impl_for_process_test("Box", impl_id, None, method_id),
        );
        seed_box_receiver_pattern(&mut mono, impl_id);

        let mut first = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("first".to_string()),
                    ty: Type::Struct {
                        id: DefId::new(CrateId(0), LocalDefId(700)),
                        args: vec![Type::I64],
                    },
                    span: crate::lexer::Span::test(),
                }),
                "map".to_string(),
                Vec::new(),
                Some(ReceiverMode::Move),
                selected_process_method_target(impl_id, method_id),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };
        let mut second = first.clone();

        mono.process_expr(&mut first);
        mono.process_expr(&mut second);

        let first_id = match &first.kind {
            HirExprKind::Call(callee, _, _) => match &callee.kind {
                HirExprKind::ResolvedVar(reference) => match reference.target {
                    HirVarTarget::Instance(id) => id,
                    _ => panic!("first call should target an instance"),
                },
                _ => panic!("first call callee should be resolved"),
            },
            _ => panic!("first method call should lower to a direct call"),
        };
        let second_id = match &second.kind {
            HirExprKind::Call(callee, _, _) => match &callee.kind {
                HirExprKind::ResolvedVar(reference) => match reference.target {
                    HirVarTarget::Instance(id) => id,
                    _ => panic!("second call should target an instance"),
                },
                _ => panic!("second call callee should be resolved"),
            },
            _ => panic!("second method call should lower to a direct call"),
        };

        assert_eq!(first_id, second_id);
    }
}

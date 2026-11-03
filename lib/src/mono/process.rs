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

    fn zero_substitution_callable_instance(&self, id: DefId) -> Option<crate::mono::InstanceId> {
        self.zero_substitution_function_instance(id)
    }

    pub(super) fn process(
        &mut self,
        mut program: super::hir_types::HirProgram,
    ) -> super::hir_types::HirProgram {
        self.language_items = program.language_items.clone();
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
            {
                self.register_concrete_function(name.clone(), func);
            } else {
                self.register_generic_function(name, func);
            }
        }

        let mut function_ids = self.concrete_functions.keys().copied().collect::<Vec<_>>();
        function_ids.sort();
        for id in function_ids {
            if let Some(func) = self.concrete_functions.remove(&id) {
                let processed = self.process_function(func);
                self.concrete_functions.insert(id, processed);
            }
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
            if Self::should_register_function_instance(&func) {
                self.register_function_instance(&name, &func);
            }
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

    pub(super) fn process_function(&mut self, mut func: HirFunction) -> HirFunction {
        let previous_owner = self.current_function_owner;
        self.current_function_owner = Some(func.id);
        self.var_types.clear();
        self.process_params(Some(func.id), &mut func.params);
        func.ret_type = self.substitute_generics(&func.ret_type);
        self.process_block(&mut func.body);
        self.current_function_owner = previous_owner;
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
        block.ty = self.substitute_generics(&block.ty);
        for stmt in &mut block.stmts {
            self.process_stmt(stmt);
        }
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

    fn process_expr(&mut self, expr: &mut HirExpr) {
        expr.ty = self.substitute_generics(&expr.ty);

        match &mut expr.kind {
            HirExprKind::Var(_) => {}
            HirExprKind::ResolvedVar(reference) => {
                if let HirVarTarget::Local(id) = reference.target {
                    if let Some(ty) = self.var_types.get(&id) {
                        expr.ty = self.type_for(*ty);
                    }
                    return;
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
                self.process_expr(func);
                for arg in args.iter_mut() {
                    self.process_expr(arg);
                }

                let generic_target = match &func.kind {
                    HirExprKind::ResolvedVar(reference) => match reference.target {
                        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                            self.generic_function_by_id(id)
                        }
                        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => None,
                    },
                    _ => None,
                };
                let fallback_name = match &func.kind {
                    HirExprKind::ResolvedVar(reference) => match reference.target {
                        HirVarTarget::Function(_) | HirVarTarget::Extern(_) => {
                            Some(reference.name.clone())
                        }
                        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => None,
                    },
                    _ => None,
                };

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
                if !had_static_authority && fallback_name.is_some() {
                    if let Some((lookup_name, generic_func)) = generic_target {
                        if let Some((instance_id, specialized_func_type, specialized_ret_type)) =
                            self.monomorphize_call(&lookup_name, &generic_func, &args_for_mono)
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
            HirExprKind::FieldAccess(inner, _, _) | HirExprKind::TupleIndex(inner, _) => {
                self.process_expr(inner);
            }
            HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
                for e in elems {
                    self.process_expr(e);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => self.process_expr(value),
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.process_expr(&mut field.value);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for e in args {
                    self.process_expr(e);
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
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.process_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.process_expr(guard);
                    }
                    self.process_block(&mut arm.body);
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
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                let old_var_types = self.var_types.clone();
                self.process_params(self.current_function_owner, params);
                for capture in captures {
                    capture.ty = self.substitute_generics(&capture.ty);
                    let ty_id = self.intern_type(&capture.ty);
                    self.var_types.insert(capture.local_id, ty_id);
                }
                self.process_block(body);
                self.var_types = old_var_types;
            }
            HirExprKind::Ref(_, inner) => {
                self.process_expr(inner);
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
mod tests;

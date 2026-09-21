use std::collections::{hash_map::Entry, HashMap};

use super::hir_types::{HirBlock, HirExpr, HirFunction, HirParam};
use crate::ids::{CrateId, DefId, LocalDefId, TypeId};
use crate::selection::{constructor_target_from_applied_type, type_pattern_matches};
use crate::type_services::normalize::apply_type_lambda;
use crate::types::{GenericParamId, Type};

use super::Monomorphizer;

impl Monomorphizer {
    fn generic_ids_for_function(func: &HirFunction) -> Vec<GenericParamId> {
        func.generic_params.iter().map(|param| param.id).collect()
    }

    /// Monomorphize a function call by creating a specialized version if needed.
    pub(super) fn monomorphize_call(
        &mut self,
        func_name: &str,
        generic_func: &HirFunction,
        args: &[HirExpr],
    ) -> Option<(crate::mono::InstanceId, Type, Type)> {
        let type_args = self.extract_type_args(generic_func, args);

        let origin = self.function_instance_origin(generic_func);
        let substitution = type_args.clone();
        let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
        if let Some(instance_id) = self.instances.get(&instance_key) {
            if let Some(body) = self.instances.pre_mir_body(instance_id) {
                let param_types: Vec<Type> = body.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    body.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(body.is_unsafe),
                );
                return Some((instance_id, func_type, body.ret_type.clone()));
            }
            let pending = self.create_specialization_signature(generic_func, &type_args);
            let param_types = pending
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect();
            let func_type = Type::function_with_safety(
                param_types,
                pending.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(pending.is_unsafe),
            );
            return Some((instance_id, func_type, pending.ret_type));
        }

        let specialized_name = format!("{}_mono_{}", func_name, self.counter);
        self.counter += 1;

        let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);
        let instance_id = self
            .instances
            .intern(instance_key, |id| crate::mono::InstanceRecord {
                id,
                origin: origin.clone(),
                substitution: substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new(func_name, backend_symbol.clone()),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
        let specialized_func =
            self.create_specialization(generic_func, &type_args, &specialized_name);
        let specialized_ret_type = specialized_func.ret_type.clone();
        let param_types: Vec<Type> = specialized_func
            .params
            .iter()
            .map(|p| p.ty.clone())
            .collect();
        let specialized_func_type = Type::function_with_safety(
            param_types,
            specialized_ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(specialized_func.is_unsafe),
        );

        self.instances
            .insert_pre_mir_body(instance_id, specialized_func.clone());
        if let Some(body) = self.instances.pre_mir_body(instance_id) {
            let param_types: Vec<Type> = body.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                body.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(body.is_unsafe),
            );
            return Some((instance_id, func_type, body.ret_type.clone()));
        }

        Some((instance_id, specialized_func_type, specialized_ret_type))
    }

    fn bounded_constructor_target(
        &self,
        generic_func: &HirFunction,
        param_ty: &Type,
        arg_ty: &Type,
    ) -> Option<(GenericParamId, Type)> {
        let Type::Apply { constructor, .. } = param_ty else {
            return None;
        };
        let Type::Generic(constructor_id) = constructor.as_ref() else {
            return None;
        };
        let bounds = generic_func.generic_bounds.get(constructor_id)?;

        let mut targets = Vec::new();
        for bound in bounds {
            let Some(impls) = self.trait_impls.get(&bound.trait_id) else {
                continue;
            };
            for imp in impls {
                if imp.trait_arg_types.len() != bound.type_args.len() {
                    continue;
                }
                let Some((target, mut impl_subst)) =
                    constructor_target_from_applied_type(&imp.receiver_pattern, arg_ty)
                else {
                    continue;
                };
                if imp
                    .trait_arg_types
                    .iter()
                    .zip(&bound.type_args)
                    .all(|(expected, actual)| {
                        type_pattern_matches(expected, actual, &mut impl_subst)
                    })
                {
                    targets.push(target);
                }
            }
        }
        targets.sort_by_key(ToString::to_string);
        targets.dedup();
        match targets.as_slice() {
            [target] => Some((*constructor_id, target.clone())),
            _ => None,
        }
    }

    /// Extract concrete type arguments from a call site
    fn extract_type_args(&mut self, generic_func: &HirFunction, args: &[HirExpr]) -> Vec<TypeId> {
        let mut substitution: HashMap<GenericParamId, TypeId> = HashMap::new();
        let param_start = if generic_func.is_method { 1 } else { 0 };
        let generic_ids = Self::generic_ids_for_function(generic_func);

        for (index, arg) in args.iter().enumerate() {
            let Some(param) = generic_func.params.get(index + param_start) else {
                continue;
            };
            let Some((constructor_id, target)) =
                self.bounded_constructor_target(generic_func, &param.ty, &arg.ty)
            else {
                continue;
            };
            if substitution.contains_key(&constructor_id) {
                continue;
            }
            let target = self.intern_type(&target);
            substitution.insert(constructor_id, target);
        }

        for (i, arg) in args.iter().enumerate() {
            if i + param_start < generic_func.params.len() {
                let param = &generic_func.params[i + param_start];
                self.extract_generics_from_type(
                    &param.ty,
                    &arg.ty,
                    &generic_ids,
                    &mut substitution,
                );
            }
        }

        // A type parameter may occur only in a bound, rather than in a value
        // parameter (C: ForEach T, A: FnMut T, ()). Recover it from the concrete
        // receiver's unique impl instead of falling back to the numeric default.
        while generic_ids.iter().any(|id| !substitution.contains_key(id)) {
            let before = substitution.len();
            for (subject, bounds) in &generic_func.generic_bounds {
                if !substitution.contains_key(subject) {
                    continue;
                }
                let actual = self.apply_substitution(&Type::Generic(*subject), &substitution);
                for bound in bounds {
                    let known = substitution
                        .keys()
                        .map(|param| {
                            (
                                *param,
                                self.apply_substitution(&Type::Generic(*param), &substitution),
                            )
                        })
                        .collect::<HashMap<_, _>>();
                    let mut candidates = Vec::new();
                    for imp in self.trait_impls.get(&bound.trait_id).into_iter().flatten() {
                        if imp.trait_arg_types.len() != bound.type_args.len() {
                            continue;
                        }
                        let Some(impl_subst) = crate::selection::impl_receiver_pattern_substitution(
                            imp,
                            &actual,
                            self.trait_impls.values().flatten(),
                        ) else {
                            continue;
                        };
                        let mut inferred = known.clone();
                        if bound.type_args.iter().zip(&imp.trait_arg_types).all(
                            |(expected, found)| {
                                type_pattern_matches(
                                    expected,
                                    &found.substitute_generics(&impl_subst),
                                    &mut inferred,
                                )
                            },
                        ) && !candidates.contains(&inferred)
                        {
                            candidates.push(inferred);
                        }
                    }
                    if candidates.len() == 1 {
                        for (param, ty) in candidates.pop().unwrap() {
                            if generic_ids.contains(&param) && !substitution.contains_key(&param) {
                                if !crate::type_services::visit::type_any(&ty, |nested| {
                                    matches!(nested, Type::Generic(_) | Type::TypeVar(_))
                                }) {
                                    substitution.insert(param, self.intern_type(&ty));
                                }
                            }
                        }
                    }
                }
            }
            if substitution.len() == before {
                break;
            }
        }

        for generic_id in &generic_ids {
            if !substitution.contains_key(generic_id) {
                for param in &generic_func.params {
                    if let Some(inferred_ty) =
                        self.extract_generic_from_type(&param.ty, *generic_id, &substitution)
                    {
                        substitution.insert(*generic_id, inferred_ty);
                        break;
                    }
                }
                if !substitution.contains_key(generic_id) {
                    if let Some(inferred_ty) = self.extract_generic_from_type(
                        &generic_func.ret_type,
                        *generic_id,
                        &substitution,
                    ) {
                        substitution.insert(*generic_id, inferred_ty);
                    }
                }
            }
        }

        let default_ty = self.intern_type(&Type::I64);
        let mut type_args = vec![default_ty; generic_func.generic_params.len()];
        for (idx, generic_id) in generic_ids.iter().enumerate() {
            type_args[idx] = substitution.get(generic_id).copied().unwrap_or(default_ty);
        }

        type_args
    }

    fn extract_generic_from_type(
        &mut self,
        ty: &Type,
        target_gen: GenericParamId,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> Option<TypeId> {
        match ty {
            Type::Generic(gen_param) if *gen_param == target_gen => {
                substitution.get(&target_gen).copied()
            }
            Type::Function {
                params: param_types,
                ret: ret_type,
                captures,
                ..
            } => {
                for param_ty in param_types {
                    if let Some(ty) =
                        self.extract_generic_from_type(param_ty, target_gen, substitution)
                    {
                        return Some(ty);
                    }
                }
                self.extract_generic_from_type(ret_type, target_gen, substitution)
                    .or_else(|| {
                        captures.iter().find_map(|capture| {
                            self.extract_generic_from_type(&capture.ty, target_gen, substitution)
                        })
                    })
            }
            Type::Slice(inner) => self.extract_generic_from_type(inner, target_gen, substitution),
            Type::Array(inner, _) => {
                self.extract_generic_from_type(inner, target_gen, substitution)
            }
            Type::Tuple(elems) => {
                for elem in elems {
                    if let Some(ty) = self.extract_generic_from_type(elem, target_gen, substitution)
                    {
                        return Some(ty);
                    }
                }
                None
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                for arg in args {
                    if let Some(ty) = self.extract_generic_from_type(arg, target_gen, substitution)
                    {
                        return Some(ty);
                    }
                }
                None
            }
            Type::Reference { inner, .. } | Type::Pointer(inner) => {
                self.extract_generic_from_type(inner, target_gen, substitution)
            }
            Type::Projection { ty, trait_args, .. } => {
                if let Some(ty) = self.extract_generic_from_type(ty, target_gen, substitution) {
                    return Some(ty);
                }
                for arg in trait_args {
                    if let Some(ty) = self.extract_generic_from_type(arg, target_gen, substitution)
                    {
                        return Some(ty);
                    }
                }
                None
            }
            Type::Apply { constructor, args } => self
                .extract_generic_from_type(constructor, target_gen, substitution)
                .or_else(|| {
                    args.iter().find_map(|arg| {
                        self.extract_generic_from_type(arg, target_gen, substitution)
                    })
                }),
            Type::Lambda { body, .. } => {
                self.extract_generic_from_type(body, target_gen, substitution)
            }
            _ => None,
        }
    }

    pub(super) fn extract_generics_from_type(
        &mut self,
        param_ty: &Type,
        arg_ty: &Type,
        generic_ids: &[GenericParamId],
        substitution: &mut HashMap<GenericParamId, TypeId>,
    ) {
        match (param_ty, arg_ty) {
            (Type::Generic(gen_param), _) if generic_ids.contains(gen_param) => {
                if !substitution.contains_key(gen_param) {
                    let substituted_arg_ty = self.apply_substitution(arg_ty, substitution);
                    let substituted_arg_ty = self.intern_type(&substituted_arg_ty);
                    substitution.insert(*gen_param, substituted_arg_ty);
                }
            }
            (
                Type::Function {
                    params: param_params,
                    ret: param_ret,
                    ..
                },
                Type::Function {
                    params: arg_params,
                    ret: arg_ret,
                    ..
                },
            ) => {
                for (p_param, a_param) in param_params.iter().zip(arg_params.iter()) {
                    self.extract_generics_from_type(p_param, a_param, generic_ids, substitution);
                }
                self.extract_generics_from_type(param_ret, arg_ret, generic_ids, substitution);
            }
            (Type::Slice(inner_param), Type::Slice(inner_arg)) => {
                self.extract_generics_from_type(inner_param, inner_arg, generic_ids, substitution);
            }
            (Type::Array(inner_param, param_len), Type::Array(inner_arg, arg_len))
                if param_len == arg_len =>
            {
                self.extract_generics_from_type(inner_param, inner_arg, generic_ids, substitution);
            }
            (Type::Tuple(elems_param), Type::Tuple(elems_arg)) => {
                for (p_elem, a_elem) in elems_param.iter().zip(elems_arg.iter()) {
                    self.extract_generics_from_type(p_elem, a_elem, generic_ids, substitution);
                }
            }
            (
                Type::Struct {
                    id: param_id,
                    args: param_args,
                },
                Type::Struct {
                    id: arg_id,
                    args: arg_args,
                },
            ) if param_id == arg_id => {
                for (p_arg, a_arg) in param_args.iter().zip(arg_args.iter()) {
                    self.extract_generics_from_type(p_arg, a_arg, generic_ids, substitution);
                }
            }
            (
                Type::Enum {
                    id: param_id,
                    args: param_args,
                },
                Type::Enum {
                    id: arg_id,
                    args: arg_args,
                },
            ) if param_id == arg_id => {
                for (p_arg, a_arg) in param_args.iter().zip(arg_args.iter()) {
                    self.extract_generics_from_type(p_arg, a_arg, generic_ids, substitution);
                }
            }
            (
                Type::Reference {
                    mutable: param_mutable,
                    inner: param_inner,
                },
                Type::Reference {
                    mutable: arg_mutable,
                    inner: arg_inner,
                },
            ) if param_mutable == arg_mutable => {
                self.extract_generics_from_type(param_inner, arg_inner, generic_ids, substitution);
            }
            (Type::Pointer(param_inner), Type::Pointer(arg_inner)) => {
                self.extract_generics_from_type(param_inner, arg_inner, generic_ids, substitution);
            }
            (
                Type::Projection {
                    ty: param_ty,
                    trait_id: param_trait_id,
                    assoc_type: param_assoc_type,
                    trait_args: param_trait_args,
                },
                Type::Projection {
                    ty: arg_ty,
                    trait_id: arg_trait_id,
                    assoc_type: arg_assoc_type,
                    trait_args: arg_trait_args,
                },
            ) if param_trait_id == arg_trait_id
                && param_assoc_type == arg_assoc_type
                && param_trait_args.len() == arg_trait_args.len() =>
            {
                self.extract_generics_from_type(param_ty, arg_ty, generic_ids, substitution);
                for (param_arg, arg_arg) in param_trait_args.iter().zip(arg_trait_args.iter()) {
                    self.extract_generics_from_type(param_arg, arg_arg, generic_ids, substitution);
                }
            }
            (
                Type::Apply {
                    constructor: param_constructor,
                    args: param_args,
                },
                Type::Apply {
                    constructor: arg_constructor,
                    args: arg_args,
                },
            ) if param_args.len() == arg_args.len() => {
                self.extract_generics_from_type(
                    param_constructor,
                    arg_constructor,
                    generic_ids,
                    substitution,
                );
                for (param_arg, arg_arg) in param_args.iter().zip(arg_args) {
                    self.extract_generics_from_type(param_arg, arg_arg, generic_ids, substitution);
                }
            }
            (
                Type::Apply {
                    constructor,
                    args: param_args,
                },
                Type::Struct { id, args } | Type::Enum { id, args },
            ) if param_args.len() <= args.len() => {
                if let Type::Generic(generic_id) = constructor.as_ref() {
                    if let Some(section_id) = substitution.get(generic_id).copied() {
                        let section = self.type_for(section_id);
                        if let Some(applied) = apply_type_lambda(&section, param_args) {
                            self.extract_generics_from_type(
                                &applied,
                                arg_ty,
                                generic_ids,
                                substitution,
                            );
                            return;
                        }
                    }
                }
                let constructor_arg_count = args.len() - param_args.len();
                let flavor = match arg_ty {
                    Type::Struct { .. } => crate::types::NominalTypeKind::Struct,
                    Type::Enum { .. } => crate::types::NominalTypeKind::Enum,
                    _ => unreachable!(),
                };
                let mut arg_constructor = Type::Constructor { id: *id, flavor };
                if constructor_arg_count > 0 {
                    arg_constructor = Type::Apply {
                        constructor: Box::new(arg_constructor),
                        args: args[..constructor_arg_count].to_vec(),
                    };
                }
                self.extract_generics_from_type(
                    constructor,
                    &arg_constructor,
                    generic_ids,
                    substitution,
                );
                for (param_arg, arg_arg) in param_args.iter().zip(&args[constructor_arg_count..]) {
                    self.extract_generics_from_type(param_arg, arg_arg, generic_ids, substitution);
                }
            }
            (
                Type::Lambda {
                    params: param_kinds,
                    body: param_body,
                },
                Type::Lambda {
                    params: arg_kinds,
                    body: arg_body,
                },
            ) if param_kinds == arg_kinds => {
                self.extract_generics_from_type(param_body, arg_body, generic_ids, substitution);
            }
            _ => {}
        }
    }

    fn apply_substitution(
        &mut self,
        ty: &Type,
        substitution: &HashMap<GenericParamId, TypeId>,
    ) -> Type {
        struct DirectSubstituter<'a> {
            monomorphizer: &'a Monomorphizer,
            substitution: &'a HashMap<GenericParamId, TypeId>,
        }

        impl crate::type_services::visit::TypeFolder for DirectSubstituter<'_> {
            fn fold_type(&mut self, ty: Type) -> Type {
                match ty {
                    Type::Generic(param) => self
                        .substitution
                        .get(&param)
                        .map(|replacement| self.monomorphizer.type_for(*replacement))
                        .unwrap_or(Type::Generic(param)),
                    other => crate::type_services::visit::fold_type_children(other, self),
                }
            }
        }

        crate::type_services::visit::fold_type(
            ty.clone(),
            &mut DirectSubstituter {
                monomorphizer: self,
                substitution,
            },
        )
    }

    fn create_specialization(
        &mut self,
        generic_func: &HirFunction,
        type_args: &[TypeId],
        specialized_name: &str,
    ) -> HirFunction {
        let signature = self.create_specialization_signature(generic_func, type_args);
        let mut substitution = HashMap::new();
        for (i, generic_id) in Self::generic_ids_for_function(generic_func)
            .iter()
            .enumerate()
        {
            if i < type_args.len() {
                substitution.insert(*generic_id, type_args[i]);
            }
        }

        let mut body = self.substitute_block(&generic_func.body, &substitution);
        let old_var_types = self.var_types.clone();
        self.var_types.clear();
        let mut params_for_body = signature.params.clone();
        self.process_params(Some(generic_func.id), &mut params_for_body);
        self.process_block(&mut body);
        self.var_types = old_var_types;

        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: specialized_name.to_string(),
            generic_params: vec![],
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params: signature.params,
            ret_type: signature.ret_type,
            body,
            is_curried: generic_func.is_curried,
            is_method: generic_func.is_method,
            self_receiver: generic_func.self_receiver,
            is_unsafe: generic_func.is_unsafe,
        }
    }

    fn create_specialization_signature(
        &mut self,
        generic_func: &HirFunction,
        type_args: &[TypeId],
    ) -> HirFunction {
        let substitution = Self::generic_ids_for_function(generic_func)
            .into_iter()
            .zip(type_args.iter().copied())
            .collect::<HashMap<_, _>>();
        let params = generic_func
            .params
            .iter()
            .map(|param| HirParam {
                name: param.name.clone(),
                local_id: param.local_id,
                ty: self.substitute_type_with_map(&param.ty, &substitution),
                mutable: param.mutable,
                is_ref: param.is_ref,
            })
            .collect();
        HirFunction {
            id: generic_func.id,
            name: generic_func.name.clone(),
            generic_params: vec![],
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type: self.substitute_type_with_map(&generic_func.ret_type, &substitution),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: generic_func.is_curried,
            is_method: generic_func.is_method,
            self_receiver: generic_func.self_receiver,
            is_unsafe: generic_func.is_unsafe,
        }
    }

    pub(super) fn contains_generic(&self, ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::Generic(_)))
    }

    pub(super) fn extract_type_args_from_expr_type(
        &mut self,
        generic_func: &HirFunction,
        concrete_ty: &Type,
    ) -> Option<Vec<TypeId>> {
        let generic_func_ty = Type::function(
            generic_func.params.iter().map(|p| p.ty.clone()).collect(),
            generic_func.ret_type.clone(),
        );

        let mut substitution = HashMap::new();
        let generic_ids = Self::generic_ids_for_function(generic_func);
        if let Type::Function {
            params: concrete_params,
            ..
        } = concrete_ty
        {
            for (param, concrete_param) in generic_func.params.iter().zip(concrete_params) {
                let Some((constructor_id, target)) =
                    self.bounded_constructor_target(generic_func, &param.ty, concrete_param)
                else {
                    continue;
                };
                if let Entry::Vacant(entry) = substitution.entry(constructor_id) {
                    let target = self.intern_type(&target);
                    entry.insert(target);
                }
            }
        }
        self.extract_generics_from_type(
            &generic_func_ty,
            concrete_ty,
            &generic_ids,
            &mut substitution,
        );

        if generic_ids.iter().any(|id| !substitution.contains_key(id)) {
            return None;
        }

        let default_ty = self.intern_type(&Type::I64);
        let mut type_args = vec![default_ty; generic_func.generic_params.len()];
        for (index, id) in generic_ids.iter().enumerate() {
            if index < type_args.len() {
                type_args[index] = substitution.get(id).copied().unwrap_or(default_ty);
            }
        }

        Some(type_args)
    }

    pub(super) fn monomorphize_with_type_args(
        &mut self,
        func_name: &str,
        generic_func: &HirFunction,
        type_args: &[TypeId],
    ) -> Option<(crate::mono::InstanceId, Type, Type)> {
        let origin = self.function_instance_origin(generic_func);
        let substitution = type_args.to_vec();
        let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
        if let Some(instance_id) = self.instances.get(&instance_key) {
            if let Some(body) = self.instances.pre_mir_body(instance_id) {
                let param_types: Vec<Type> = body.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    body.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(body.is_unsafe),
                );
                return Some((instance_id, func_type, body.ret_type.clone()));
            }
        }

        let specialized_name = format!("{}_mono_{}", func_name, self.counter);
        self.counter += 1;

        let specialized_func =
            self.create_specialization(generic_func, type_args, &specialized_name);
        let specialized_ret_type = specialized_func.ret_type.clone();
        let param_types: Vec<Type> = specialized_func
            .params
            .iter()
            .map(|p| p.ty.clone())
            .collect();
        let specialized_func_type = Type::function_with_safety(
            param_types,
            specialized_ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(specialized_func.is_unsafe),
        );

        let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);
        let instance_id = self
            .instances
            .intern(instance_key, |id| crate::mono::InstanceRecord {
                id,
                origin: origin.clone(),
                substitution: substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new(func_name, backend_symbol.clone()),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
        self.instances
            .insert_pre_mir_body(instance_id, specialized_func.clone());
        if let Some(body) = self.instances.pre_mir_body(instance_id) {
            let param_types: Vec<Type> = body.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                body.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(body.is_unsafe),
            );
            return Some((instance_id, func_type, body.ret_type.clone()));
        }

        Some((instance_id, specialized_func_type, specialized_ret_type))
    }

    /// Substitute generic types in the current context.
    pub(super) fn substitute_generics(&mut self, ty: &Type) -> Type {
        ty.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use crate::hir::HirParam;
    use crate::ids::{AssocTypeId, CrateId, LocalDefId};
    use crate::mono::hir_types::{
        HirBlock, HirExpr, HirExprKind, HirImpl, HirImplOwner, HirImplReceiverPattern,
    };
    use crate::type_services::kind::Kind;
    use crate::type_services::normalize::TypeNormalizationEnv;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, NominalTypeKind};

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn generic_function(id: DefId, param_ty: Type, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: "id".to_string(),
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
                ty: param_ty,
                mutable: false,
                is_ref: false,
            }],
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

    fn expr_with_type(ty: Type) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty,
            span: crate::lexer::Span::test(),
        }
    }

    #[test]
    fn extract_type_args_uses_bound_impl_constructor_section() {
        let mut mono = Monomorphizer::new();
        let function_id = def_id(1);
        let trait_id = def_id(2);
        let impl_id = def_id(3);
        let result_id = def_id(4);
        let f = GenericParamDecl::new(
            GenericParamId {
                owner: function_id,
                index: 0,
            },
            "F",
            Kind::arrow(Kind::Type, Kind::Type),
        );
        let a = GenericParamDecl::type_param(
            GenericParamId {
                owner: function_id,
                index: 1,
            },
            "A",
        );
        let function = HirFunction {
            id: function_id,
            name: "map_op".to_string(),
            generic_params: vec![f.clone(), a.clone()],
            generic_bounds: HashMap::from([(
                f.id,
                vec![crate::types::TraitBound {
                    trait_id,
                    type_args: Vec::new(),
                }],
            )])
            .into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Apply {
                    constructor: Box::new(Type::Generic(f.id)),
                    args: vec![Type::Generic(a.id)],
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let impl_error = GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "E",
        );
        let section = |error| Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Enum {
                id: result_id,
                args: vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    error,
                ],
            }),
        };
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("ResultSection".to_string()),
                type_name: "ResultSection".to_string(),
                type_generics: vec![impl_error.clone()],
                receiver_pattern: HirImplReceiverPattern::Constructor(section(Type::Generic(
                    impl_error.id,
                ))),
                trait_name: Some("Functor".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::new(),
            }],
        );

        let type_args = mono.extract_type_args(
            &function,
            &[expr_with_type(Type::Enum {
                id: result_id,
                args: vec![Type::I64, Type::Bool],
            })],
        );

        assert_eq!(mono.type_for(type_args[0]), section(Type::Bool));
        assert_eq!(mono.type_for(type_args[1]), Type::I64);

        let function_value_args = mono
            .extract_type_args_from_expr_type(
                &function,
                &Type::function(
                    vec![Type::Enum {
                        id: result_id,
                        args: vec![Type::I64, Type::Bool],
                    }],
                    Type::Unit,
                ),
            )
            .expect("concrete function value should infer all generic arguments");
        assert_eq!(mono.type_for(function_value_args[0]), section(Type::Bool));
        assert_eq!(mono.type_for(function_value_args[1]), Type::I64);
    }

    #[test]
    fn monomorphize_call_returns_instance_id_for_new_specialization() {
        let mut mono = Monomorphizer::new();
        let id = def_id(10);
        let generic = generic_function(
            id,
            Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
            Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
        );
        mono.resolver.item_paths.insert("identity".to_string(), id);
        let args = vec![HirExpr {
            kind: HirExprKind::IntLiteral(21),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        }];

        let (instance_id, func_ty, ret_ty) = mono
            .monomorphize_call("identity", &generic, &args)
            .expect("generic call should specialize");

        assert_eq!(ret_ty, Type::I64);
        assert_eq!(func_ty, Type::function(vec![Type::I64], Type::I64));
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(record.origin, crate::mono::InstanceOrigin::Function(id));
        assert_eq!(record.substitution.len(), 1);
        assert_eq!(mono.type_for(record.substitution[0]), Type::I64);
    }

    #[test]
    fn generic_ids_for_function_uses_function_owner_not_foreign_occurrences() {
        let function_owner = def_id(10);
        let foreign_owner = def_id(20);
        let generic_func = generic_function(
            function_owner,
            Type::Generic(GenericParamId {
                owner: foreign_owner,
                index: 0,
            }),
            Type::I64,
        );

        let generic_ids = Monomorphizer::generic_ids_for_function(&generic_func);

        assert_eq!(
            generic_ids,
            vec![GenericParamId {
                owner: function_owner,
                index: 0,
            }]
        );
    }

    #[test]
    fn contains_generic_detects_projection_generic_inputs() {
        let mono = Monomorphizer::new();
        let generic_id = GenericParamId {
            owner: def_id(10),
            index: 0,
        };

        assert!(mono.contains_generic(&Type::Projection {
            ty: Box::new(Type::Generic(generic_id)),
            trait_id: def_id(1),
            assoc_type: AssociatedTypeKey {
                owner: def_id(1),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: Vec::new(),
        }));
        assert!(mono.contains_generic(&Type::Projection {
            ty: Box::new(Type::I64),
            trait_id: def_id(1),
            assoc_type: AssociatedTypeKey {
                owner: def_id(1),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(generic_id)],
        }));
    }

    #[test]
    fn extract_type_args_ignores_same_index_foreign_generic_owner() {
        let mut mono = Monomorphizer::new();
        let function_owner = def_id(10);
        let foreign_owner = def_id(20);
        let generic_func = generic_function(
            function_owner,
            Type::Generic(GenericParamId {
                owner: foreign_owner,
                index: 0,
            }),
            Type::Generic(GenericParamId {
                owner: function_owner,
                index: 0,
            }),
        );

        let type_args = mono.extract_type_args(&generic_func, &[expr_with_type(Type::Bool)]);

        assert_eq!(
            type_args
                .into_iter()
                .map(|ty_id| mono.type_for(ty_id))
                .collect::<Vec<_>>(),
            vec![Type::I64]
        );
    }

    #[test]
    fn extract_type_args_does_not_infer_across_projection_identity_mismatch() {
        let mut mono = Monomorphizer::new();
        let function_owner = def_id(10);
        let generic_id = GenericParamId {
            owner: function_owner,
            index: 0,
        };
        let generic_func = generic_function(
            function_owner,
            Type::Projection {
                ty: Box::new(Type::Generic(generic_id)),
                trait_id: def_id(1),
                assoc_type: AssociatedTypeKey {
                    owner: def_id(1),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: vec![Type::Generic(generic_id)],
            },
            Type::I64,
        );

        let type_args = mono.extract_type_args(
            &generic_func,
            &[expr_with_type(Type::Projection {
                ty: Box::new(Type::Bool),
                trait_id: def_id(2),
                assoc_type: AssociatedTypeKey {
                    owner: def_id(2),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: vec![Type::Bool],
            })],
        );

        assert_eq!(
            type_args
                .into_iter()
                .map(|ty_id| mono.type_for(ty_id))
                .collect::<Vec<_>>(),
            vec![Type::I64]
        );
    }

    #[test]
    fn constructor_specialization_normalizes_nested_generic_application() {
        let function_id = def_id(30);
        let option_id = def_id(31);
        let constructor_param = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let element_param = GenericParamId {
            owner: function_id,
            index: 1,
        };
        let unary = Kind::arrow(Kind::Type, Kind::Type);
        let applied = Type::Apply {
            constructor: Box::new(Type::Generic(constructor_param)),
            args: vec![Type::Generic(element_param)],
        };
        let generic = HirFunction {
            id: function_id,
            name: "lift".to_string(),
            generic_params: vec![
                GenericParamDecl::new(constructor_param, "F", unary.clone()),
                GenericParamDecl::type_param(element_param, "A"),
            ],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: applied.clone(),
                mutable: false,
                is_ref: false,
            }],
            ret_type: applied,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let concrete = Type::Enum {
            id: option_id,
            args: vec![Type::I64],
        };
        let mut env = TypeNormalizationEnv::new();
        env.register_constructor(option_id, NominalTypeKind::Enum, unary);
        env.register_generic_kind(constructor_param, Kind::arrow(Kind::Type, Kind::Type));
        env.register_generic_kind(element_param, Kind::Type);
        let mut mono = Monomorphizer::new();
        mono.normalization_env = env;

        let (instance, _, ret) = mono
            .monomorphize_call("lift", &generic, &[expr_with_type(concrete.clone())])
            .expect("constructor application should specialize");
        let record = mono.instances.record(instance).unwrap();

        assert_eq!(
            mono.type_for(record.substitution[0]),
            Type::Constructor {
                id: option_id,
                flavor: NominalTypeKind::Enum,
            }
        );
        assert_eq!(mono.type_for(record.substitution[1]), Type::I64);
        assert_eq!(ret, concrete);
        assert_eq!(
            mono.instances.pre_mir_body(instance).unwrap().params[0].ty,
            concrete
        );
    }

    #[test]
    fn specialization_beta_reduces_result_section_after_error_substitution() {
        let function_id = def_id(40);
        let result_id = def_id(41);
        let error_param = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let section = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Enum {
                id: result_id,
                args: vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    Type::Generic(error_param),
                ],
            }),
        };
        let application = Type::Apply {
            constructor: Box::new(section),
            args: vec![Type::I64],
        };
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(result_id, NominalTypeKind::Enum, 2);
        env.register_generic_kind(error_param, Kind::Type);
        let mut mono = Monomorphizer::new();
        mono.normalization_env = env;
        let bool_id = mono.intern_type(&Type::Bool);

        assert_eq!(
            mono.substitute_type_with_map(&application, &HashMap::from([(error_param, bool_id)])),
            Type::Enum {
                id: result_id,
                args: vec![Type::I64, Type::Bool],
            }
        );
    }
}

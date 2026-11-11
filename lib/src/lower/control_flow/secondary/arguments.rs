//! Call-argument application for secondary expressions.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::intrinsics::{
    infer_intrinsic_arg_types, infer_intrinsic_return_type, is_intrinsic_name,
};
use crate::lower::Lowerer;
use crate::types::{GenericParamId, TraitBound, Type};
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn apply_arguments_secondary(
        &mut self,
        expr: HirExpr,
        args: &[ast::Argument],
        span: Span,
    ) -> HirExpr {
        let mut expr = expr;
        let mut hir_args: Vec<HirExpr>;

        if matches!(self.engine.resolve(&expr.ty), Type::Error) {
            return HirExpr {
                ty: Type::Error,
                kind: expr.kind.clone(),
                span,
            };
        }

        if let HirExprKind::FieldAccess(recv, method_name, _) = &expr.kind {
            let recv_ty = self.engine.resolve(&recv.ty);
            let found_method = self.concrete_method_candidate((**recv).clone(), method_name);

            let found_method = found_method.or_else(|| {
                if let Type::TypeVar(var_id) = &recv_ty {
                    let bounds = self.engine.get_bounds(*var_id);
                    if bounds.is_empty() {
                        return None;
                    }
                    let receiver_candidates = self.receiver_adjustment_candidates((**recv).clone());
                    let result = self
                        .selection_service()
                        .select_bound_method_preferring_non_ref_receiver(
                            &receiver_candidates,
                            &bounds,
                            method_name,
                            recv_ty.clone(),
                            matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                        );
                    if let Some(selected) = self.handle_optional_selection(result, span.clone()) {
                        return Some(selected);
                    }
                }
                if let Type::Generic(gen_param) = &recv_ty {
                    let bounds = self
                        .current_impl_bounds()
                        .get(gen_param)
                        .cloned()
                        .unwrap_or_default();
                    let receiver_candidates = self.receiver_adjustment_candidates((**recv).clone());
                    let result = self
                        .selection_service()
                        .select_bound_method_preferring_non_ref_receiver(
                            &receiver_candidates,
                            &bounds,
                            method_name,
                            recv_ty.clone(),
                            matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                        );
                    if let Some(selected) = self.handle_optional_selection(result, span.clone()) {
                        return Some(selected);
                    }
                }
                if let Type::Reference { inner, .. } = &recv_ty {
                    if let Type::Generic(gen_param) = inner.as_ref() {
                        let bounds = self
                            .current_impl_bounds()
                            .get(gen_param)
                            .cloned()
                            .unwrap_or_default();
                        let receiver_candidates =
                            self.receiver_adjustment_candidates((**recv).clone());
                        let result = self
                            .selection_service()
                            .select_bound_method_preferring_non_ref_receiver(
                                &receiver_candidates,
                                &bounds,
                                method_name,
                                inner.as_ref().clone(),
                                matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                            );
                        if let Some(selected) = self.handle_optional_selection(result, span.clone())
                        {
                            return Some(selected);
                        }
                    }
                }
                None
            });

            if let Some(selected) = found_method {
                hir_args = args
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        self.lower_selected_method_argument(&argument.arg, &selected, index)
                    })
                    .collect();
                if hir_args.is_empty() && !selected.substituted_params.is_empty() {
                    self.diagnostics.push_type_with_span(
                        format!(
                            "Type mismatch: method '{}' expected {} args, got 0",
                            method_name,
                            selected.substituted_params.len()
                        ),
                        span.clone(),
                    );
                    return self.error_expression_at(span.clone());
                }

                let adjusted_recv = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );
                let Some((method_func, ret_ty, coerced_args, target)) =
                    self.selected_method_call_types(&selected, hir_args)
                else {
                    return self.error_expression_at(span.clone());
                };
                if let Some(method_id) = target.method_id() {
                    self.record_source_reference(
                        span.clone(),
                        crate::source_map::SourceSymbol::Definition(method_id),
                    );
                }
                hir_args = coerced_args;

                if method_func.is_unsafe && !self.is_in_unsafe() {
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "Call to unsafe function '{}' requires an unsafe block",
                            method_name
                        ),
                        span.clone(),
                    );
                }

                if method_func.is_curried && !hir_args.is_empty() {
                    let mut applied = HirExpr {
                        ty: ret_ty,
                        kind: HirExprKind::MethodCall(
                            Box::new(adjusted_recv.clone()),
                            method_name.clone(),
                            vec![hir_args[0].clone()],
                            method_func.self_receiver,
                            Some(target),
                        ),
                        span: span.clone(),
                    };

                    if hir_args.len() > 1 {
                        applied =
                            self.fold_unary_calls(applied, hir_args[1..].to_vec(), span.clone());
                    }

                    return applied;
                }

                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::MethodCall(
                        Box::new(adjusted_recv),
                        method_name.clone(),
                        hir_args,
                        method_func.self_receiver,
                        Some(target),
                    ),
                    span,
                };
            }
        }

        let callable_selection = if matches!(self.engine.resolve(&expr.ty), Type::Function { .. })
            || matches!(expr.kind, HirExprKind::EnumVariant(..))
        {
            None
        } else {
            self.callable_method_candidate(expr.clone(), None)
        };
        let expected_param_types = match self.engine.resolve(&expr.ty) {
            Type::Function { params, .. } => Some(params),
            _ => callable_selection
                .as_ref()
                .and_then(|selected| selected.substituted_params.first())
                .map(|param| match (&param.ty, args.len()) {
                    (Type::Unit, 0) => Vec::new(),
                    (Type::Tuple(types), count) if count > 1 => types.clone(),
                    (ty, _) => vec![ty.clone()],
                }),
        };
        hir_args = Vec::with_capacity(args.len());
        for (index, argument) in args.iter().enumerate() {
            let expected = expected_param_types
                .as_ref()
                .and_then(|params| params.get(index))
                .cloned();
            let hir_arg = if let Some(lambda) = Self::lambda_from_expression(&argument.arg) {
                let mut expected_params = expected
                    .as_ref()
                    .and_then(|ty| self.callable_argument_parameters(ty));
                if lambda.parameters.len() == 1
                    && !expected
                        .as_ref()
                        .is_some_and(|ty| matches!(self.engine.resolve(ty), Type::Function { .. }))
                {
                    if let Some(params) = &mut expected_params {
                        if params.len() > 1 {
                            *params = vec![Type::Tuple(std::mem::take(params))];
                        }
                    }
                }
                self.lower_lambda_with_expected_params(lambda, expected_params.as_deref())
            } else {
                self.lower_expression(&argument.arg)
            };
            if args.len() == 1 {
                self.infer_static_trait_argument_coercions(&expr, std::slice::from_ref(&hir_arg));
            }
            hir_args.push(match expected {
                Some(expected) => {
                    let hir_arg = self.coerce_argument_to_expected(hir_arg, &expected);
                    self.propagate_call_argument_bounds(&expected, &hir_arg.ty);
                    hir_arg
                }
                None => hir_arg,
            });
        }

        if matches!(&expr.kind, HirExprKind::MethodCall(_, _, _, _, _)) && hir_args.is_empty() {
            return expr;
        }

        if let HirExprKind::EnumVariant(enum_name, variant_name, _, variant_location) = &expr.kind {
            let enum_name = enum_name.clone();
            let variant_name = variant_name.clone();
            let variant_location = variant_location.clone();
            let enum_id = variant_location
                .as_ref()
                .map(|location| location.owner)
                .or_else(|| {
                    crate::lower::resolution::LowerResolutionContext::new(self)
                        .resolve_enum_type(&enum_name)
                        .map(|enum_info| enum_info.id)
                });
            let Some(enum_id) = enum_id else {
                return self.error_expression_at(span.clone());
            };
            let type_args = if let Some(enum_info) = self.items.enumeration(enum_id).cloned() {
                if !enum_info.generic_params.is_empty() {
                    let mut type_var_mapping: HashMap<GenericParamId, Type> = HashMap::new();
                    for (index, _) in enum_info.generic_params.iter().enumerate() {
                        let tv = self.engine.fresh_type_var_at(span.clone());
                        type_var_mapping.insert(
                            GenericParamId {
                                owner: enum_info.id,
                                index: index as u32,
                            },
                            tv,
                        );
                    }
                    if let Some(variant) =
                        enum_info.variants.iter().find(|v| v.name == variant_name)
                    {
                        if let HirVariantFields::Positional(field_types) = &variant.fields {
                            for (arg, field_ty) in hir_args.iter().zip(field_types.iter()) {
                                let expected = field_ty.substitute_generics(&type_var_mapping);
                                let _ = self.engine.unify(&arg.ty, &expected);
                            }
                        }
                    }
                    enum_info
                        .generic_params
                        .iter()
                        .enumerate()
                        .map(|(index, _)| {
                            if let Some(tv) = type_var_mapping.get(&GenericParamId {
                                owner: enum_info.id,
                                index: index as u32,
                            }) {
                                self.engine.resolve(tv)
                            } else {
                                Type::Error
                            }
                        })
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };
            return HirExpr {
                ty: Type::Enum {
                    id: enum_id,
                    args: type_args,
                },
                kind: HirExprKind::EnumVariant(enum_name, variant_name, hir_args, variant_location),
                span,
            };
        }

        if let HirExprKind::Var(name) = &expr.kind {
            if is_intrinsic_name(name) {
                if name == "OverflowChecks" {
                    if !hir_args.is_empty() {
                        self.diagnostics.push_type_with_span(
                            "OverflowChecks expects no arguments".to_string(),
                            span.clone(),
                        );
                    }
                    return HirExpr {
                        ty: Type::Bool,
                        kind: HirExprKind::BoolLiteral(self.overflow_checks),
                        span,
                    };
                }
                if let Some(integer) = crate::mir::integer::IntegerIntrinsic::from_name(name) {
                    if !matches!(
                        integer.op,
                        crate::mir::integer::IntegerOp::Shl | crate::mir::integer::IntegerOp::Shr
                    ) {
                        if let [left, right] = hir_args.as_slice() {
                            if self.engine.unify(&left.ty, &right.ty).is_err() {
                                self.diagnostics.push_type_with_span(
                                    format!("{name} requires integer operands of the same type"),
                                    span.clone(),
                                );
                            }
                        }
                    }
                }
                let unsafe_intrinsics = [
                    "PtrOffset",
                    "DropInPlace",
                    "AtomicU64Exchange",
                    "AtomicU64FetchAdd",
                    "AtomicU64FetchSub",
                    "AtomicU64Store",
                ];
                if unsafe_intrinsics.contains(&name.as_str()) && !self.is_in_unsafe() {
                    self.diagnostics.push_selection_with_span(
                        format!("Intrinsic '~{}' requires an unsafe block", name),
                        span.clone(),
                    );
                }
                let expected_arg_types = infer_intrinsic_arg_types(name);
                let is_atomic = name.starts_with("AtomicU64");
                if is_atomic && hir_args.len() != expected_arg_types.len() {
                    self.diagnostics.push_with_span(
                        format!(
                            "Intrinsic '~{}' expects exactly {} arguments, found {}",
                            name,
                            expected_arg_types.len(),
                            hir_args.len()
                        ),
                        span.clone(),
                    );
                }
                for (arg, expected_ty) in hir_args.iter().zip(expected_arg_types.iter()) {
                    if self.engine.unify(&arg.ty, expected_ty).is_err() {
                        let expected = self.display_type(expected_ty);
                        self.diagnostics.push_type_with_span(
                            format!("Intrinsic '~{}' expected argument type {}", name, expected),
                            arg.span.clone(),
                        );
                    }
                    if is_atomic {
                        let resolved = self.engine.resolve(&arg.ty);
                        if resolved != *expected_ty {
                            let expected = self.display_type(expected_ty);
                            let found = self.display_type(&resolved);
                            self.diagnostics.push_with_span(
                                format!(
                                    "Intrinsic '~{}' expected argument type {}, found {}",
                                    name, expected, found
                                ),
                                arg.span.clone(),
                            );
                        }
                    }
                }
                let mut arrptr_elem_ty: Option<Type> = None;
                let mut intrinsic_ty_error = false;
                if name == "PtrFromParts" {
                    if hir_args.len() != 2 {
                        self.diagnostics.push_type_with_span(
                            "PtrFromParts requires exactly two arguments".to_string(),
                            span.clone(),
                        );
                    } else {
                        let element = self.engine.fresh_type_var_at(span.clone());
                        for (arg, expected) in hir_args
                            .iter()
                            .zip([Type::Pointer(Box::new(element)), Type::I64])
                        {
                            if self.engine.unify(&arg.ty, &expected).is_err() {
                                self.diagnostics.push_type_with_span(
                                    "PtrFromParts requires a data pointer and I64 length"
                                        .to_string(),
                                    arg.span.clone(),
                                );
                            }
                        }
                    }
                    return HirExpr {
                        ty: self.engine.fresh_type_var_at(span.clone()),
                        kind: HirExprKind::Intrinsic {
                            name: name.clone(),
                            args: hir_args,
                        },
                        span,
                    };
                }
                if (name == "ArrPtr" || name == "ArrayLen") && !hir_args.is_empty() {
                    let resolved_arg_ty =
                        self.resolve_projection_type(&self.engine.resolve(&hir_args[0].ty));

                    match &resolved_arg_ty {
                        Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Str) => {
                            if name == "ArrPtr" {
                                arrptr_elem_ty = Some(Type::U8);
                            }
                        }
                        Type::Reference { inner, .. }
                            if matches!(inner.as_ref(), Type::Slice(_) | Type::Array(_, _)) =>
                        {
                            if name == "ArrPtr" {
                                if let Type::Slice(element) | Type::Array(element, _) =
                                    inner.as_ref()
                                {
                                    arrptr_elem_ty = Some(element.as_ref().clone());
                                }
                            }
                        }
                        Type::Array(_, _) if name == "ArrPtr" => {
                            let actual = self.display_type(&resolved_arg_ty);
                            self.diagnostics.push_with_span(
                                format!("ArrPtr expected slice, got {}", actual),
                                hir_args[0].span.clone(),
                            );
                            intrinsic_ty_error = true;
                        }
                        _ => {
                            let elem_ty = self.engine.fresh_type_var_at(span.clone());
                            let array_ty = Type::Slice(Box::new(elem_ty.clone()));
                            let _ = self.engine.unify(&hir_args[0].ty, &array_ty);
                            if name == "ArrPtr" {
                                arrptr_elem_ty = Some(elem_ty);
                            }
                        }
                    }
                }
                if name == "DropInPlace" {
                    if hir_args.len() != 1 {
                        self.diagnostics.push_type_with_span(
                            "DropInPlace requires exactly one argument".to_string(),
                            span.clone(),
                        );
                        intrinsic_ty_error = true;
                    } else {
                        let pointee_ty = self.engine.fresh_type_var_at(span.clone());
                        let ptr_ty = Type::Pointer(Box::new(pointee_ty));
                        if self.engine.unify(&hir_args[0].ty, &ptr_ty).is_err() {
                            let resolved_arg_ty =
                                self.resolve_projection_type(&self.engine.resolve(&hir_args[0].ty));
                            let actual = self.display_type(&resolved_arg_ty);
                            self.diagnostics.push_type_with_span(
                                format!("DropInPlace expected raw pointer, got {}", actual),
                                hir_args[0].span.clone(),
                            );
                            intrinsic_ty_error = true;
                        }
                    }
                }
                let ret_ty = if intrinsic_ty_error {
                    Type::Error
                } else if let Some(elem_ty) = arrptr_elem_ty {
                    Type::Pointer(Box::new(elem_ty))
                } else {
                    infer_intrinsic_return_type(name, &hir_args)
                };
                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::Intrinsic {
                        name: name.clone(),
                        args: hir_args,
                    },
                    span,
                };
            }
        }

        if let HirExprKind::Var(func_name) = &expr.kind {
            let function_id = self.resolve_module_alias_or_item_def_id(func_name);
            if let Some(func) = function_id.and_then(|id| self.items.function(id)).cloned() {
                if func.is_unsafe && !self.is_in_unsafe() {
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "Call to unsafe function '{}' requires an unsafe block",
                            func_name
                        ),
                        span.clone(),
                    );
                }
                if !func.generic_params.is_empty() && !matches!(expr.ty, Type::Function { .. }) {
                    expr.ty = self.instantiate_function_type(&func, expr.span.clone());
                }
            }
        }
        if let HirExprKind::ResolvedVar(reference) = &expr.kind {
            if let HirVarTarget::Function(function_id) = reference.target {
                if let Some(func) = self.current_function(function_id).cloned() {
                    if func.is_unsafe && !self.is_in_unsafe() {
                        self.diagnostics.push_selection_with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                reference.name
                            ),
                            span.clone(),
                        );
                    }
                    if !func.generic_params.is_empty() && !matches!(expr.ty, Type::Function { .. })
                    {
                        expr.ty = self.instantiate_function_type(&func, expr.span.clone());
                    }
                }
            }
        }

        let resolved_expr_ty = self.engine.resolve(&expr.ty);
        if let Some(selected) = callable_selection {
            return self.lower_callable_call(selected, hir_args);
        }
        let defer_argument_coercions = matches!(resolved_expr_ty, Type::TypeVar(_));
        if let Type::Function { params, .. } = &resolved_expr_ty {
            if params.len() == 1 && hir_args.len() > 1 {
                return self.fold_unary_calls(expr, hir_args, span);
            }

            let mut args_iter = hir_args.into_iter();
            let mut coerced_args = Vec::new();

            for param in params {
                let Some(arg) = args_iter.next() else {
                    break;
                };
                coerced_args.push(self.coerce_argument_to_expected(arg, param));
            }

            coerced_args.extend(args_iter);
            hir_args = coerced_args;

            if params.len() != hir_args.len() {
                self.diagnostics.push_selection_with_span(
                    format!(
                        "Type mismatch: function expected {} args, got {}",
                        params.len(),
                        hir_args.len()
                    ),
                    span.clone(),
                );
                let target = self.call_target_for_callee(&expr);
                return HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Call(Box::new(expr), hir_args, target),
                    span,
                };
            }
        }

        let ret_ty = self.engine.fresh_type_var_at(span.clone());
        let arg_types: Vec<Type> = hir_args
            .iter()
            .map(|arg| {
                if defer_argument_coercions
                    && (matches!(expr.kind, HirExprKind::FieldAccess(..))
                        || self.language_items.fn_once.is_none())
                    && matches!(
                        self.engine.resolve(&arg.ty),
                        Type::Reference { inner, .. }
                            if matches!(inner.as_ref(), Type::Array(_, _))
                    )
                {
                    let expected = self.engine.fresh_type_var_at(arg.span.clone());
                    self.constraint_store.add_coercion(
                        arg.ty.clone(),
                        expected.clone(),
                        arg.span.clone(),
                        "deferred callable argument coercion",
                    );
                    expected
                } else {
                    arg.ty.clone()
                }
            })
            .collect();
        let safety = match self.resolve_projection_type(&self.engine.resolve(&expr.ty)) {
            Type::Function { safety, .. } => safety,
            _ => crate::types::FunctionSafety::Safe,
        };
        let expected_fn_ty = Type::function_with_safety(arg_types, ret_ty.clone(), safety);
        let mut callable_ty = self.engine.resolve(&expr.ty);
        while let Type::Reference { inner, .. } = callable_ty {
            callable_ty = *inner;
        }
        let numeric_callee = if let Type::TypeVar(id) = callable_ty {
            self.constraint_store
                .literal_default_type_for_representative(id, |var| {
                    match self.engine.resolve(&Type::TypeVar(var)) {
                        Type::TypeVar(id) => Some(id),
                        _ => None,
                    }
                })
                .is_some()
        } else {
            false
        };
        if !numeric_callee && !matches!(expr.kind, HirExprKind::FieldAccess(..)) {
            if let Type::TypeVar(id) = callable_ty {
                let protocol = if self.expr_is_mutable_lvalue(&expr) {
                    self.language_items
                        .fn_mut
                        .as_ref()
                        .map(|items| items.trait_id)
                } else {
                    let binding_scope = match &expr.kind {
                        HirExprKind::Var(name) => self.scope.binding_scope_index(name),
                        HirExprKind::ResolvedVar(reference) => self.scope.binding_scope_index(&reference.name),
                        _ => None,
                    };
                    if binding_scope.is_some_and(|scope| self.body.requires_reusable_callable(scope)) {
                        // Reading a captured callable, or calling across a
                        // loop backedge, requires reusable shared invocation.
                        self.language_items.fn_trait.as_ref().map(|items| items.trait_id)
                    } else {
                        None
                    }
                }
                .or_else(|| {
                    self.language_items
                        .fn_once
                        .as_ref()
                        .map(|items| items.trait_id)
                });
                if let Some(trait_id) = protocol {
                    let args_ty = match hir_args.as_slice() {
                        [] => Type::Unit,
                        [arg] => arg.ty.clone(),
                        args => Type::Tuple(args.iter().map(|arg| arg.ty.clone()).collect()),
                    };
                    let bound = TraitBound {
                        trait_id,
                        type_args: vec![args_ty, ret_ty.clone()],
                    };
                    self.engine.add_bound(id, bound.clone());
                    self.constraint_store.add_trait(
                        callable_ty.clone(),
                        bound,
                        span.clone(),
                        "function call",
                    );
                    if let Some(selected) = self.callable_method_candidate(expr.clone(), None) {
                        return self.lower_callable_call(selected, hir_args);
                    }
                    return self.error_expression_at(span);
                }
            }
        }
        if (numeric_callee || self.engine.unify(&callable_ty, &expected_fn_ty).is_err())
            && !matches!(expr.kind, HirExprKind::FieldAccess(..))
        {
            let message = format!(
                "value of type {} is not callable with these arguments",
                self.display_type(&callable_ty)
            );
            self.diagnostics.push_type_with_span(message, span.clone());
            return self.error_expression_at(span);
        }
        let resolved_ret_ty = self.engine.resolve(&ret_ty);
        self.report_unsafe_function_value_call_from_type(&expr);

        HirExpr {
            ty: resolved_ret_ty,
            kind: {
                let target = self.call_target_for_callee(&expr);
                HirExprKind::Call(Box::new(expr), hir_args, target)
            },
            span,
        }
    }
}

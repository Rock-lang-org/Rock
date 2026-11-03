//! Identifier path lowering.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::{path_names, seg_name, Lowerer};
use crate::types::Type;
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn full_path_span(path: &[ast::IdentOrType]) -> Span {
        let first = path.first().expect("path must not be empty");
        let last = path.last().expect("path must not be empty");
        let start = match first {
            ast::IdentOrType::Ident(ident) => ident.span.start,
            ast::IdentOrType::Type(ty) => ty.span().start,
        };
        let end = match last {
            ast::IdentOrType::Ident(ident) => ident.span.end,
            ast::IdentOrType::Type(ty) => ty.span().end,
        };
        let file_path = match first {
            ast::IdentOrType::Ident(ident) => ident.span.file_path.clone(),
            ast::IdentOrType::Type(ty) => ty.span().file_path,
        };
        Span::new(file_path, start, end)
    }

    fn record_var_reference(&mut self, span: Span, target: &HirVarTarget) {
        let symbol = match target {
            HirVarTarget::Local(local) => {
                self.current_body_def_id()
                    .map(|owner| crate::source_map::SourceSymbol::Local {
                        owner,
                        local: *local,
                    })
            }
            HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                Some(crate::source_map::SourceSymbol::Definition(*id))
            }
            HirVarTarget::Instance(_) => None,
        };
        if let Some(symbol) = symbol {
            self.record_source_reference(span, symbol);
        }
    }

    /// Replace semantic generic parameters in `ty` with fresh TypeVars in the current engine.
    /// This is called at every function reference site so generic functions can be
    /// independently instantiated per call (like HM polymorphism).
    pub(crate) fn instantiate_generics(&mut self, ty: Type, span: Span) -> Type {
        let mut generic_params = std::collections::HashSet::new();
        ty.collect_generic_params(&mut generic_params);
        if generic_params.is_empty() {
            return ty;
        }
        let subst: HashMap<crate::types::GenericParamId, Type> = generic_params
            .into_iter()
            .map(|param| {
                let kind = self
                    .engine
                    .kind_of(&Type::Generic(param))
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                (
                    param,
                    self.engine.fresh_type_var_at_kind(span.clone(), kind),
                )
            })
            .collect();
        ty.substitute_generics(&subst)
    }

    pub(crate) fn lower_identifier_path(&mut self, path: &ast::IdentifierPath) -> HirExpr {
        let span = Self::full_path_span(&path.path);
        // Simple case: single identifier
        if path.path.len() == 1 {
            if let Some(name) = seg_name(&path.path[0]) {
                let name = &name;

                if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_identifier_value(name)
                {
                    let target = resolved.target.clone();
                    let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                    if let Some(target) = target.as_ref() {
                        self.record_var_reference(span.clone(), target);
                    }
                    let kind = if let Some(target) = target {
                        HirExprKind::ResolvedVar(HirVarRef {
                            name: resolved.name,
                            target,
                        })
                    } else {
                        HirExprKind::Var(resolved.name)
                    };
                    return HirExpr { ty, kind, span };
                }

                // Check if it's an enum variant (no args)
                let enum_entries: Vec<_> = self
                    .items
                    .enumerations()
                    .map(|(id, enum_info)| {
                        (
                            self.canonical_name_for_def_id(id)
                                .map(str::to_string)
                                .unwrap_or_else(|| enum_info.name.clone()),
                            enum_info.clone(),
                        )
                    })
                    .collect();
                for (enum_name, enum_info) in &enum_entries {
                    for variant in &enum_info.variants {
                        if variant.name == *name {
                            if matches!(variant.fields, HirVariantFields::Unit) {
                                self.record_source_reference(
                                    span.clone(),
                                    crate::source_map::SourceSymbol::Variant {
                                        owner: enum_info.id,
                                        variant: variant.id,
                                    },
                                );
                                // For generic enums, create fresh type vars
                                let type_args = self
                                    .engine
                                    .fresh_generic_args(&enum_info.generic_params, &span);
                                return HirExpr {
                                    ty: Type::Enum {
                                        id: enum_info.id,
                                        args: type_args,
                                    },
                                    kind: HirExprKind::EnumVariant(
                                        enum_name.to_string(),
                                        variant.name.clone(),
                                        vec![],
                                        Some(HirVariantLocation {
                                            owner: enum_info.id,
                                            variant_id: variant.id,
                                            name: variant.name.clone(),
                                        }),
                                    ),
                                    span,
                                };
                            }
                        }
                    }
                }

                // Check if it's a struct type name used as a value (e.g. String.from)
                // Struct names are not valid standalone values — use `Type::method` syntax
                if crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_struct_type(name)
                    .is_some()
                {
                    self.diagnostics.push_with_span(
                        format!("Type '{}' cannot be used as a value. Use '{}::method' syntax to call associated functions", name, name),
                        span.clone(),
                    );
                    return HirExpr {
                        ty: Type::Error,
                        kind: HirExprKind::Var(name.clone()),
                        span: span.clone(),
                    };
                }

                // Unknown variable - emit an error and use Error type to stop cascade errors
                self.diagnostics
                    .push_with_span(format!("Unknown variable: {}", name), span.clone());
                return HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Var(name.clone()),
                    span,
                };
            }
        }

        // Multi-segment path (e.g., Crate::func, Module::func, or Enum::Variant)
        let names = path_names(&path.path);

        if names.len() == 3 {
            match self.resolve_static_qualified_bound_method_path(
                &names[0],
                &names[1],
                &names[2],
                span.clone(),
            ) {
                Ok(Some((ty, target))) => {
                    let Some(method_id) = target.method.method_id() else {
                        self.diagnostics.push_with_span(
                            "selected qualified constructor bound has no member identity"
                                .to_string(),
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    };
                    let callee = HirExpr {
                        ty,
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: names.join("::"),
                            target: HirVarTarget::Function(method_id),
                        }),
                        span: span.clone(),
                    };
                    self.record_source_reference(
                        span.clone(),
                        crate::source_map::SourceSymbol::Definition(method_id),
                    );
                    return self.static_method_value_lambda(callee, target);
                }
                Ok(None) => {}
                Err(message) if self.current_generic_param_id_for_name(&names[0]).is_some() => {
                    self.diagnostics.push_with_span(message, span.clone());
                    return self.error_expression_at(span.clone());
                }
                Err(_) => {}
            }

            if let Some(target_ty) = self.constructor_target_for_path_segment(&path.path[0]) {
                if let Some(trait_id) = self.trait_by_name(&names[1]).map(|trait_def| trait_def.id)
                {
                    match self.select_constructor_static_member(
                        &target_ty,
                        Some(trait_id),
                        &names[2],
                        span.clone(),
                    ) {
                        Ok(Some(selected)) => {
                            match self.instantiate_selected_constructor_member(
                                target_ty,
                                selected,
                                names.join("::"),
                                span.clone(),
                            ) {
                                Ok((callee, target)) => {
                                    return self.static_method_value_lambda(callee, target)
                                }
                                Err(message) => {
                                    self.diagnostics.push_with_span(message, span.clone());
                                    return self.error_expression_at(span.clone());
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(message) => {
                            self.diagnostics.push_with_span(message, span.clone());
                            return self.error_expression_at(span.clone());
                        }
                    }
                }
            }
        }

        if names.len() == 2 {
            // Check for Enum::Variant first
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_enum_variant_path(&names)
            {
                let enum_info = resolved.owner;
                let variant = resolved.variant;
                self.record_source_reference(
                    span.clone(),
                    crate::source_map::SourceSymbol::Variant {
                        owner: enum_info.id,
                        variant: variant.id,
                    },
                );
                // For generic enums, create fresh type vars for type params
                let type_args = self
                    .engine
                    .fresh_generic_args(&enum_info.generic_params, &span);
                let ty = Type::Enum {
                    id: enum_info.id,
                    args: type_args,
                };
                return HirExpr {
                    ty,
                    kind: HirExprKind::EnumVariant(
                        resolved.owner_name,
                        variant.name.clone(),
                        vec![],
                        Some(HirVariantLocation {
                            owner: enum_info.id,
                            variant_id: variant.id,
                            name: variant.name.clone(),
                        }),
                    ),
                    span,
                };
            }

            match self.resolve_static_bound_method_path(&names[0], &names[1], span.clone()) {
                Ok(Some((ty, target))) => {
                    let Some(method_id) = target.method.method_id() else {
                        self.diagnostics.push_with_span(
                            "selected static bound method has no member identity".to_string(),
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    };
                    let callee = HirExpr {
                        ty,
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: names.join("::"),
                            target: HirVarTarget::Function(method_id),
                        }),
                        span: span.clone(),
                    };
                    self.record_source_reference(
                        span.clone(),
                        crate::source_map::SourceSymbol::Definition(method_id),
                    );
                    return self.static_method_value_lambda(callee, target);
                }
                Ok(None) => {}
                Err(message) => {
                    self.diagnostics
                        .push_selection_with_span(message, span.clone());
                    return self.error_expression_at(span.clone());
                }
            }

            // Inherent associated functions retain precedence over trait members.
            let mut static_method_error = None;
            match crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_static_method_path(&names)
            {
                Ok(Some(resolved)) => match self
                    .instantiate_static_method_value(resolved, span.clone())
                {
                    Ok((callee, target)) => return self.static_method_value_lambda(callee, target),
                    Err(message) => {
                        self.diagnostics
                            .push_selection_with_span(message, span.clone());
                        return self.error_expression_at(span.clone());
                    }
                },
                Ok(None) => {}
                Err(message) => {
                    static_method_error = Some(message);
                }
            }

            if let Some(target_ty) = self.constructor_target_for_path_segment(&path.path[0]) {
                match self.select_constructor_static_member(
                    &target_ty,
                    None,
                    &names[1],
                    span.clone(),
                ) {
                    Ok(Some(selected)) => {
                        match self.instantiate_selected_constructor_member(
                            target_ty,
                            selected,
                            names.join("::"),
                            span.clone(),
                        ) {
                            Ok((callee, target)) => {
                                return self.static_method_value_lambda(callee, target)
                            }
                            Err(message) => {
                                self.diagnostics
                                    .push_selection_with_span(message, span.clone());
                                return self.error_expression_at(span.clone());
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(message) => {
                        self.diagnostics
                            .push_selection_with_span(message, span.clone());
                        return self.error_expression_at(span.clone());
                    }
                }
            }

            if let Some(message) = static_method_error {
                self.diagnostics.push_with_span(message, span.clone());
                return self.error_expression_at(span.clone());
            }

            // Check for Crate::Function or Module::Function
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_qualified_value_path(&names)
            {
                let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                if let Some(target) = resolved.target.as_ref() {
                    self.record_var_reference(span.clone(), target);
                }
                let kind = if let Some(target) = resolved.target {
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: resolved.name,
                        target,
                    })
                } else {
                    HirExprKind::Var(resolved.name)
                };
                return HirExpr { ty, kind, span };
            }
        }

        // Handle longer paths (e.g., crate::module::function)
        if names.len() > 2 {
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_qualified_value_path(&names)
            {
                let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                if let Some(target) = resolved.target.as_ref() {
                    self.record_var_reference(span.clone(), target);
                }
                let kind = if let Some(target) = resolved.target {
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: resolved.name,
                        target,
                    })
                } else {
                    HirExprKind::Var(resolved.name)
                };
                return HirExpr { ty, kind, span };
            }
        }

        // Check if the first segment looks like an unexported struct/type
        // (exists under a qualified name but not under the short name)
        if !names.is_empty() {
            let type_name = &names[0];
            if crate::lower::resolution::LowerResolutionContext::new(self)
                .struct_segment_looks_unexported(type_name)
            {
                self.diagnostics.push_with_span(
                    format!("Type '{}' is not publicly exported", type_name),
                    span.clone(),
                );
                return HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Var(names.join("::")),
                    span,
                };
            }
        }

        // Fallback: use the full path as a name
        let full_name = names.join("::");
        let ty = self.engine.fresh_type_var_at(span.clone());
        HirExpr {
            ty,
            kind: HirExprKind::Var(full_name),
            span,
        }
    }
}

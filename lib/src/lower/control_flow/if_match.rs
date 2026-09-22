use crate::ast;
use crate::hir::*;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use crate::lower::Lowerer;

impl Lowerer {
    pub(crate) fn lower_if(&mut self, if_expr: &ast::If) -> HirExpr {
        let condition = self.lower_expression(&if_expr.condition.expression);
        let span = condition.span.clone();
        let _ = self.engine.unify(&condition.ty, &Type::Bool);

        self.push_scope();
        let then_branch = self.lower_block(&if_expr.then);
        self.pop_scope();

        let else_branch = if_expr.else_.as_ref().map(|else_| match else_ {
            ast::Else::Block(block) => {
                self.push_scope();
                let b = self.lower_block(block);
                self.pop_scope();
                b
            }
            ast::Else::If(nested_if) => {
                let nested = self.lower_if(nested_if);
                HirBlock {
                    ty: nested.ty.clone(),
                    stmts: vec![HirStmt::Expr(nested)],
                }
            }
        });

        let result_ty = if let Some(ref else_b) = else_branch {
            self.merge_control_flow_types(&then_branch.ty, &else_b.ty, &span)
        } else {
            Type::Unit
        };

        HirExpr {
            ty: result_ty,
            kind: HirExprKind::If {
                condition: Box::new(condition),
                then_branch,
                else_branch,
            },
            span,
        }
    }

    pub(crate) fn lower_match(&mut self, match_expr: &ast::Match) -> HirExpr {
        let mut scrutinee = self.lower_expression(&match_expr.expr);
        let span = scrutinee.span.clone();
        scrutinee.ty = match self.engine.normalize_resolved_type(&scrutinee.ty) {
            Ok(ty) => ty,
            Err(error) => {
                self.diagnostics.push_type_with_span(error, span.clone());
                Type::Error
            }
        };
        let mut result_ty: Option<Type> = None;

        let arms: Vec<HirMatchArm> = match_expr
            .arms
            .iter()
            .map(|arm| {
                self.push_scope();

                let pattern = self.lower_pattern(&arm.pattern, &scrutinee.ty);
                self.validate_enum_payload_pattern_support(
                    &pattern,
                    &scrutinee.ty,
                    arm.condition.is_some(),
                    &span,
                );
                let guard = arm.condition.as_ref().map(|c| self.lower_expression(c));

                let body = self.lower_block(&arm.body);
                result_ty = Some(match result_ty.take() {
                    Some(current) => self.merge_control_flow_types(&current, &body.ty, &span),
                    None => body.ty.clone(),
                });

                self.pop_scope();

                HirMatchArm {
                    pattern,
                    guard,
                    body,
                }
            })
            .collect();

        HirExpr {
            ty: result_ty
                .map(|ty| self.engine.resolve(&ty))
                .unwrap_or(Type::Unit),
            kind: HirExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        }
    }

    fn merge_control_flow_types(
        &mut self,
        left: &Type,
        right: &Type,
        span: &crate::lexer::Span,
    ) -> Type {
        self.merge_control_flow_types_with_variance(left, right, true, span)
    }

    fn merge_control_flow_types_with_variance(
        &mut self,
        left: &Type,
        right: &Type,
        covariant: bool,
        span: &crate::lexer::Span,
    ) -> Type {
        let left = self.resolve_projection_type(&self.engine.resolve(left));
        let right = self.resolve_projection_type(&self.engine.resolve(right));

        match (&left, &right) {
            (Type::TypeVar(_), _) => {
                let merged = Self::make_function_slots_conservative(&right, covariant);
                let _ = self.engine.unify(&left, &merged);
                self.engine.resolve(&merged)
            }
            (_, Type::TypeVar(_)) => {
                let merged = Self::make_function_slots_conservative(&left, covariant);
                let _ = self.engine.unify(&right, &merged);
                self.engine.resolve(&merged)
            }
            (
                Type::Function {
                    params: left_params,
                    ret: left_ret,
                    safety: left_safety,
                    callable_kind: left_kind,
                    captures: left_captures,
                },
                Type::Function {
                    params: right_params,
                    ret: right_ret,
                    safety: right_safety,
                    callable_kind: right_kind,
                    captures: right_captures,
                },
            ) if left_params.len() == right_params.len() => {
                let params = left_params
                    .iter()
                    .zip(right_params.iter())
                    .map(|(left_param, right_param)| {
                        self.merge_control_flow_types_with_variance(
                            left_param,
                            right_param,
                            !covariant,
                            span,
                        )
                    })
                    .collect();
                let ret = self
                    .merge_control_flow_types_with_variance(left_ret, right_ret, covariant, span);
                let mut captures = left_captures.clone();
                for capture in right_captures {
                    if !captures.contains(capture) {
                        captures.push(capture.clone());
                    }
                }
                Type::Function {
                    params,
                    ret: Box::new(ret),
                    safety: Self::merge_function_safety(*left_safety, *right_safety, covariant),
                    callable_kind: std::cmp::max(*left_kind, *right_kind),
                    captures,
                }
            }
            (Type::Tuple(left_elems), Type::Tuple(right_elems))
                if left_elems.len() == right_elems.len() =>
            {
                Type::Tuple(
                    left_elems
                        .iter()
                        .zip(right_elems.iter())
                        .map(|(left_elem, right_elem)| {
                            self.merge_control_flow_types_with_variance(
                                left_elem, right_elem, covariant, span,
                            )
                        })
                        .collect(),
                )
            }
            (Type::Slice(left_elem), Type::Slice(right_elem)) => Type::Slice(Box::new(
                self.merge_control_flow_types_with_variance(left_elem, right_elem, covariant, span),
            )),
            (Type::Pointer(left_elem), Type::Pointer(right_elem)) => Type::Pointer(Box::new(
                self.merge_invariant_control_flow_types(left_elem, right_elem, span),
            )),
            (Type::Array(left_elem, left_len), Type::Array(right_elem, right_len))
                if left_len == right_len =>
            {
                Type::Array(
                    Box::new(self.merge_control_flow_types_with_variance(
                        left_elem, right_elem, covariant, span,
                    )),
                    *left_len,
                )
            }
            (
                Type::Reference {
                    mutable: left_mutable,
                    inner: left_inner,
                },
                Type::Reference {
                    mutable: right_mutable,
                    inner: right_inner,
                },
            ) if left_mutable == right_mutable => {
                let inner = if *left_mutable {
                    self.merge_invariant_control_flow_types(left_inner, right_inner, span)
                } else {
                    self.merge_control_flow_types_with_variance(
                        left_inner,
                        right_inner,
                        covariant,
                        span,
                    )
                };
                Type::Reference {
                    mutable: *left_mutable,
                    inner: Box::new(inner),
                }
            }
            (Type::Reference { .. }, Type::Reference { .. }) => {
                let left = self.display_type(&left);
                let right = self.display_type(&right);
                self.diagnostics.push_type_with_span(
                    format!("Type mismatch: {} vs {}", left, right),
                    span.clone(),
                );
                Type::Error
            }
            (
                Type::Struct {
                    id: left_id,
                    args: left_args,
                },
                Type::Struct {
                    id: right_id,
                    args: right_args,
                },
            ) if left_id == right_id && left_args.len() == right_args.len() => Type::Struct {
                id: *left_id,
                args: left_args
                    .iter()
                    .zip(right_args.iter())
                    .map(|(left_arg, right_arg)| {
                        self.merge_control_flow_types_with_variance(
                            left_arg, right_arg, covariant, span,
                        )
                    })
                    .collect(),
            },
            (
                Type::Enum {
                    id: left_id,
                    args: left_args,
                },
                Type::Enum {
                    id: right_id,
                    args: right_args,
                },
            ) if left_id == right_id && left_args.len() == right_args.len() => Type::Enum {
                id: *left_id,
                args: left_args
                    .iter()
                    .zip(right_args.iter())
                    .map(|(left_arg, right_arg)| {
                        self.merge_control_flow_types_with_variance(
                            left_arg, right_arg, covariant, span,
                        )
                    })
                    .collect(),
            },
            _ => {
                let mut probe = self.engine.clone_for_probe();
                if probe.unify(&left, &right).is_ok() {
                    *self.engine = probe;
                    self.engine.resolve(&left)
                } else {
                    let mut probe = self.engine.clone_for_probe();
                    if probe.unify(&right, &left).is_ok() {
                        *self.engine = probe;
                        self.engine.resolve(&right)
                    } else {
                        self.engine.resolve(&left)
                    }
                }
            }
        }
    }

    fn merge_invariant_control_flow_types(
        &mut self,
        left: &Type,
        right: &Type,
        span: &crate::lexer::Span,
    ) -> Type {
        if self.engine.unify_invariant(left, right).is_ok() {
            self.engine.resolve(left)
        } else {
            let left = self.display_type(left);
            let right = self.display_type(right);
            self.diagnostics.push_type_with_span(
                format!("Type mismatch: {} vs {}", left, right),
                span.clone(),
            );
            Type::Error
        }
    }

    fn merge_function_safety(
        left: crate::types::FunctionSafety,
        right: crate::types::FunctionSafety,
        covariant: bool,
    ) -> crate::types::FunctionSafety {
        if covariant {
            left.join(right)
        } else if matches!(
            (left, right),
            (
                crate::types::FunctionSafety::Safe,
                crate::types::FunctionSafety::Safe | crate::types::FunctionSafety::Unsafe
            ) | (
                crate::types::FunctionSafety::Unsafe,
                crate::types::FunctionSafety::Safe
            )
        ) {
            crate::types::FunctionSafety::Safe
        } else {
            crate::types::FunctionSafety::Unsafe
        }
    }

    fn make_function_slots_conservative(ty: &Type, covariant: bool) -> Type {
        match ty {
            Type::Function {
                params,
                ret,
                callable_kind,
                captures,
                ..
            } => Type::Function {
                params: params
                    .iter()
                    .map(|param| Self::make_function_slots_conservative(param, !covariant))
                    .collect(),
                ret: Box::new(Self::make_function_slots_conservative(ret, covariant)),
                safety: if covariant {
                    crate::types::FunctionSafety::Unsafe
                } else {
                    crate::types::FunctionSafety::Safe
                },
                callable_kind: *callable_kind,
                captures: captures.clone(),
            },
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|elem| Self::make_function_slots_conservative(elem, covariant))
                    .collect(),
            ),
            Type::Slice(elem) => Type::Slice(Box::new(Self::make_function_slots_conservative(
                elem, covariant,
            ))),
            Type::Pointer(elem) => Type::Pointer(elem.clone()),
            Type::Array(elem, len) => Type::Array(
                Box::new(Self::make_function_slots_conservative(elem, covariant)),
                *len,
            ),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: if *mutable {
                    inner.clone()
                } else {
                    Box::new(Self::make_function_slots_conservative(inner, covariant))
                },
            },
            Type::Struct { id, args } => Type::Struct {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| Self::make_function_slots_conservative(arg, covariant))
                    .collect(),
            },
            Type::Enum { id, args } => Type::Enum {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| Self::make_function_slots_conservative(arg, covariant))
                    .collect(),
            },
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Type::Projection {
                ty: Box::new(Self::make_function_slots_conservative(ty, covariant)),
                trait_id: *trait_id,
                assoc_type: *assoc_type,
                trait_args: trait_args
                    .iter()
                    .map(|arg| Self::make_function_slots_conservative(arg, covariant))
                    .collect(),
            },
            _ => ty.clone(),
        }
    }

    fn validate_enum_payload_pattern_support(
        &mut self,
        pattern: &HirPattern,
        source_ty: &Type,
        guarded: bool,
        span: &crate::lexer::Span,
    ) {
        match pattern {
            HirPattern::Enum(_, _, Some(location), subpatterns) => {
                let field_types =
                    self.enum_payload_field_types(location.owner, location.variant_id);
                let generic_subst = match source_ty {
                    Type::Enum { args, .. } => field_types
                        .iter()
                        .flat_map(|field_ty| {
                            let mut params = std::collections::HashSet::new();
                            field_ty.collect_generic_params(&mut params);
                            params
                        })
                        .filter_map(|param| {
                            args.get(param.index as usize)
                                .cloned()
                                .map(|arg| (param, arg))
                        })
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };

                for (subpattern, field_ty) in subpatterns.iter().zip(field_types.iter()) {
                    let field_ty = field_ty.substitute_generics(&generic_subst);
                    if Self::enum_payload_pattern_has_unsupported_literal(subpattern) {
                        self.diagnostics.push_with_span(
                            "unsupported enum payload literal pattern".to_string(),
                            span.clone(),
                        );
                    }
                    if guarded
                        && Self::pattern_binds_payload(subpattern)
                        && !TypeFacts::is_copy(&field_ty)
                    {
                        self.diagnostics.push_with_span(
                            "unsupported guarded non-copy enum payload binding".to_string(),
                            span.clone(),
                        );
                    }
                    self.validate_enum_payload_pattern_support(
                        subpattern, &field_ty, guarded, span,
                    );
                }
            }
            HirPattern::Tuple(fields) => {
                if let Type::Tuple(field_types) = source_ty {
                    for (subpattern, field_ty) in fields.iter().zip(field_types.iter()) {
                        self.validate_enum_payload_pattern_support(
                            subpattern, field_ty, guarded, span,
                        );
                    }
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    self.validate_enum_payload_pattern_support(
                        &field.pattern,
                        source_ty,
                        guarded,
                        span,
                    );
                }
            }
            HirPattern::Or(patterns) => {
                for pattern in patterns {
                    self.validate_enum_payload_pattern_support(pattern, source_ty, guarded, span);
                }
            }
            HirPattern::Enum(_, _, None, subpatterns) => {
                for subpattern in subpatterns {
                    self.validate_enum_payload_pattern_support(
                        subpattern, source_ty, guarded, span,
                    );
                }
            }
            HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
        }
    }

    fn enum_payload_field_types(
        &self,
        enum_id: crate::ids::DefId,
        variant_id: crate::ids::VariantId,
    ) -> Vec<Type> {
        let Some(enumeration) = self.items.enumeration(enum_id) else {
            return Vec::new();
        };
        let Some(variant) = enumeration
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)
        else {
            return Vec::new();
        };
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                fields.iter().map(|field| field.ty.clone()).collect()
            }
            HirVariantFields::Positional(fields) => fields.clone(),
            HirVariantFields::Unit => Vec::new(),
        }
    }

    fn enum_payload_pattern_has_unsupported_literal(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Literal(HirLiteralPattern::Float(_) | HirLiteralPattern::String(_)) => true,
            HirPattern::Tuple(fields) | HirPattern::Or(fields) => fields
                .iter()
                .any(Self::enum_payload_pattern_has_unsupported_literal),
            HirPattern::Struct(_, _, _, fields) => fields
                .iter()
                .any(|field| Self::enum_payload_pattern_has_unsupported_literal(&field.pattern)),
            HirPattern::Enum(_, _, _, fields) => fields
                .iter()
                .any(Self::enum_payload_pattern_has_unsupported_literal),
            HirPattern::Wildcard
            | HirPattern::Binding { .. }
            | HirPattern::Literal(
                HirLiteralPattern::Int(_) | HirLiteralPattern::Bool(_) | HirLiteralPattern::Char(_),
            ) => false,
        }
    }

    fn pattern_binds_payload(pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Binding { .. } => true,
            HirPattern::Tuple(fields) | HirPattern::Or(fields) => {
                fields.iter().any(Self::pattern_binds_payload)
            }
            HirPattern::Struct(_, _, _, fields) => fields
                .iter()
                .any(|field| Self::pattern_binds_payload(&field.pattern)),
            HirPattern::Enum(_, _, _, fields) => fields.iter().any(Self::pattern_binds_payload),
            HirPattern::Wildcard | HirPattern::Literal(_) => false,
        }
    }
}

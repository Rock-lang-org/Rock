use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::{ArrayLen, Type};

impl Lowerer {
    /// Array projections use MIR's pattern binder, including rest ranges.
    /// A synthetic one-arm match returns the bindings as a tuple, which the
    /// enclosing block names without extending the arm's binding scope.
    pub(crate) fn push_array_pattern_bindings(
        &mut self,
        pattern: &HirPattern,
        place: HirExpr,
        span: &crate::lexer::Span,
        stmts: &mut Vec<HirStmt>,
    ) {
        let mut inner_pattern = pattern.clone();
        let mut bindings = Vec::new();
        self.array_binding_results(&mut inner_pattern, span, &mut bindings);
        let types: Vec<_> = bindings
            .iter()
            .map(
                |(_, _, _, value): &(String, crate::ids::HirLocalId, bool, HirExpr)| {
                    value.ty.clone()
                },
            )
            .collect();
        let result_ty = Type::Tuple(types.clone());
        let body = HirBlock {
            ty: result_ty.clone(),
            stmts: vec![HirStmt::Expr(HirExpr {
                ty: result_ty.clone(),
                kind: HirExprKind::TupleLiteral(
                    bindings
                        .iter()
                        .map(|(_, _, _, value)| value.clone())
                        .collect(),
                ),
                span: span.clone(),
            })],
        };
        let value = HirExpr {
            ty: result_ty.clone(),
            kind: HirExprKind::Match {
                scrutinee: Box::new(place),
                arms: vec![HirMatchArm {
                    pattern: inner_pattern,
                    guard: None,
                    body,
                }],
            },
            span: span.clone(),
        };
        let id = self.fresh_local_id();
        let name = format!("__array_bindings_{}", self.body.fresh_pattern_temp_id());
        stmts.push(HirStmt::Let {
            name: name.clone(),
            local_id: id,
            ty: result_ty.clone(),
            value,
            mutable: false,
        });
        for (index, ((binding, local_id, mutable, _), ty)) in
            bindings.into_iter().zip(types).enumerate()
        {
            let base = HirExpr {
                ty: result_ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: name.clone(),
                    target: HirVarTarget::Local(id),
                }),
                span: span.clone(),
            };
            stmts.push(HirStmt::Let {
                name: binding,
                local_id,
                mutable,
                ty: ty.clone(),
                value: HirExpr {
                    ty,
                    kind: HirExprKind::TupleIndex(Box::new(base), index as u32),
                    span: span.clone(),
                },
            });
        }
    }

    fn array_binding_results(
        &mut self,
        pattern: &mut HirPattern,
        span: &crate::lexer::Span,
        results: &mut Vec<(String, crate::ids::HirLocalId, bool, HirExpr)>,
    ) {
        match pattern {
            HirPattern::Binding {
                name,
                local_id,
                mutable,
                ..
            } => {
                let Some(binding) = self.scope.lookup(name).cloned() else {
                    return;
                };
                let outer = *local_id;
                *local_id = self.fresh_local_id();
                results.push((
                    name.clone(),
                    outer,
                    *mutable,
                    HirExpr {
                        ty: self.engine.resolve(&binding.ty),
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: name.clone(),
                            target: HirVarTarget::Local(*local_id),
                        }),
                        span: span.clone(),
                    },
                ));
            }
            HirPattern::Deref(inner) => self.array_binding_results(inner, span, results),
            HirPattern::Array { patterns, .. }
            | HirPattern::Tuple(patterns)
            | HirPattern::Enum(_, _, _, patterns)
            | HirPattern::Or(patterns) => {
                for pattern in patterns {
                    self.array_binding_results(pattern, span, results);
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    self.array_binding_results(&mut field.pattern, span, results);
                }
            }
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
        }
    }

    pub(super) fn lower_array_pattern(
        &mut self,
        pattern: &ast::Pattern,
        elements: &[ast::ArrayPattern],
        expected: &Type,
        mode: HirBindingMode,
    ) -> HirPattern {
        let span =
            Self::pattern_binding_span(pattern).or_else(|| self.engine.span_for_type(expected));
        let rests: Vec<_> = elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| {
                matches!(element, ast::ArrayPattern::Rest(_)).then_some(index)
            })
            .collect();
        if rests.len() > 1 {
            if let Some(span) = span {
                self.diagnostics.push_type_with_span(
                    "an array pattern may contain only one rest pattern".to_string(),
                    span,
                );
            }
            return HirPattern::Wildcard;
        }
        let rest = rests.first().copied();
        let required = elements.len() - usize::from(rest.is_some());
        let resolved = self.engine.resolve(expected);
        let (element, length) = match resolved {
            Type::Array(element, length) => (*element, Some(length)),
            Type::Slice(element) => (*element, None),
            Type::TypeVar(_) => {
                let element = span
                    .clone()
                    .map(|span| self.engine.fresh_type_var_at(span))
                    .unwrap_or_else(|| self.engine.fresh_type_var());
                // Without a rest, the shape determines the array's length.
                // With a rest the input must supply an array/slice type.
                if rest.is_some() {
                    if let Some(span) = span {
                        self.diagnostics.push_type_with_span(
                            "array pattern with a rest needs a known array or slice type"
                                .to_string(),
                            span,
                        );
                    }
                    return HirPattern::Wildcard;
                }
                let length = ArrayLen::Value(required as u64);
                let _ = self
                    .engine
                    .unify(expected, &Type::Array(Box::new(element.clone()), length));
                (element, Some(length))
            }
            actual => {
                if let Some(span) = span {
                    let actual = self.display_type(&actual);
                    self.diagnostics.push_type_with_span(
                        format!("array pattern requires an array or slice, got {actual}"),
                        span,
                    );
                }
                return HirPattern::Wildcard;
            }
        };
        let known_length = match length {
            Some(ArrayLen::Value(length)) => Some(length),
            _ => None,
        };
        let irrefutable = known_length.is_some_and(|length| {
            if rest.is_some() {
                length >= required as u64
            } else {
                length == required as u64
            }
        }) || (required == 0 && rest.is_some());
        let rest_ty = match known_length {
            Some(length) => Type::Array(
                Box::new(element.clone()),
                ArrayLen::Value(length.saturating_sub(required as u64)),
            ),
            None => Type::Slice(Box::new(element.clone())),
        };
        let patterns = elements
            .iter()
            .map(|part| match part {
                ast::ArrayPattern::Pattern(pattern) => {
                    self.lower_pattern_in_mode(pattern, &element, mode)
                }
                ast::ArrayPattern::Rest(ident) if ident.name.name == "_" => HirPattern::Wildcard,
                ast::ArrayPattern::Rest(ident) => self.lower_pattern_in_mode(
                    &ast::Pattern {
                        binding: None,
                        kind: ast::PatternKind::Ident(ident.clone()),
                    },
                    &rest_ty,
                    mode,
                ),
            })
            .collect();
        HirPattern::Array {
            patterns,
            rest,
            irrefutable,
        }
    }
}

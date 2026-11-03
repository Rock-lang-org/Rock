//! Primary expression and operand lowering.

use super::operand::ExprUse;
use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(crate) fn lower_primary_expr(&mut self, primary: &ast::PrimaryExpr) -> HirExpr {
        self.lower_primary_expr_with_use(primary, ExprUse::Value)
    }

    pub(crate) fn lower_primary_expr_with_use(
        &mut self,
        primary: &ast::PrimaryExpr,
        use_kind: ExprUse,
    ) -> HirExpr {
        if let Some(lambda) = Self::desugar_call_holes(primary) {
            return self.lower_lambda(&lambda);
        }

        let mut result = if use_kind == ExprUse::Value {
            self.lower_operand(&primary.operand)
        } else {
            self.lower_operand_with_use(&primary.operand, use_kind)
        };

        // Apply type annotation if present
        if let Some(ann) = &primary.type_annotation {
            self.source_map.record_type_annotation(ann.span());
            let ann_ty = self.lower_parse_type(ann);
            if let Err(e) = self.engine.unify(&result.ty, &ann_ty) {
                self.diagnostics.push_type_with_span(
                    format!("Type annotation mismatch: {}", e.render(&self.engine)),
                    ann.span(),
                );
            }
            result.ty = ann_ty;
        }

        // Apply secondaries (function calls, field access, indexing)
        if let Some(secondaries) = &primary.secondaries {
            for (index, secondary) in secondaries.iter().enumerate() {
                let secondary_use = if use_kind == ExprUse::AssignmentPlace
                    && secondaries[index..].iter().all(|secondary| {
                        matches!(
                            secondary,
                            ast::SecondaryExpr::Dot(_) | ast::SecondaryExpr::Indice(_)
                        )
                    }) {
                    ExprUse::AssignmentPlace
                } else {
                    ExprUse::Value
                };
                let method_as_value = matches!(secondary, ast::SecondaryExpr::Dot(_))
                    && secondary_use == ExprUse::Value
                    && !matches!(
                        secondaries.get(index + 1),
                        Some(ast::SecondaryExpr::Arguments(_))
                    );
                result = self.apply_secondary(result, secondary, secondary_use, method_as_value);
            }
        }

        result
    }

    fn desugar_call_holes(primary: &ast::PrimaryExpr) -> Option<ast::LambdaDecl> {
        let mut rewritten = primary.clone();
        let mut parameters = Vec::new();

        for secondary in rewritten.secondaries.iter_mut().flatten() {
            let ast::SecondaryExpr::Arguments(arguments) = secondary else {
                continue;
            };

            for argument in arguments {
                let Some(span) = Self::call_hole_span(&argument.arg).cloned() else {
                    continue;
                };
                let name = format!("<call-hole-{}>", parameters.len());
                let ident = ast::Ident {
                    name: name.clone(),
                    span: span.clone(),
                };

                parameters.push(ast::Pattern {
                    binding: None,
                    kind: ast::PatternKind::Ident(ast::IdentPattern {
                        name: ident.clone(),
                        mut_: false,
                    }),
                });
                argument.arg =
                    ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(ast::PrimaryExpr {
                        operand: ast::Operand::Ident(ast::IdentifierPath {
                            path: vec![ast::IdentOrType::Ident(ident)],
                        }),
                        secondaries: None,
                        type_annotation: None,
                    }));
            }
        }

        if parameters.is_empty() {
            return None;
        }

        let span = parameters
            .first()
            .and_then(Self::pattern_binding_span)
            .expect("call-hole lambda has a generated parameter span");
        Some(ast::LambdaDecl {
            parameters,
            body: ast::Block {
                statements: vec![ast::Statement::Expression(ast::Expression::UnaryExpr(
                    ast::UnaryExpr::PrimaryExpr(rewritten),
                ))],
            },
            arrow_kind: ast::LambdaArrowKind::Normal,
            span,
        })
    }

    fn call_hole_span(expression: &ast::Expression) -> Option<&crate::lexer::Span> {
        let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary)) = expression else {
            return None;
        };
        if primary.secondaries.is_some() || primary.type_annotation.is_some() {
            return None;
        }
        match &primary.operand {
            ast::Operand::CallHole(span) => Some(span),
            _ => None,
        }
    }

    pub(crate) fn lower_operand(&mut self, operand: &ast::Operand) -> HirExpr {
        self.lower_operand_with_use(operand, ExprUse::Value)
    }

    fn lower_operand_with_use(&mut self, operand: &ast::Operand, use_kind: ExprUse) -> HirExpr {
        match operand {
            ast::Operand::Literal(lit) => self.lower_literal(lit),
            ast::Operand::Ident(path) => self.lower_identifier_path(path),
            ast::Operand::CallHole(span) => {
                self.diagnostics.push_with_span(
                    "Call hole '_' is only allowed as a complete function-call argument"
                        .to_string(),
                    span.clone(),
                );
                self.error_expression_at(span.clone())
            }
            ast::Operand::SelfIdent(ident) => {
                let self_ty = self
                    .scope
                    .lookup("self")
                    .map(|binding| binding.ty.clone())
                    .unwrap_or(Type::Error);
                let self_var = HirExpr {
                    ty: self_ty.clone(),
                    kind: HirExprKind::Var("self".to_string()),
                    span: ident.span.clone(),
                };

                if ident.name == "self" {
                    return self_var;
                }

                // @field -> self.field
                let mut field_ty = self.engine.fresh_type_var_at(ident.span.clone());
                let resolved_self = self.engine.resolve(&self_ty);
                let (base_expr, base_ty) = if let Type::Reference { inner, .. } = &resolved_self {
                    let deref_expr = HirExpr {
                        ty: inner.as_ref().clone(),
                        kind: HirExprKind::Deref(Box::new(self_var)),
                        span: ident.span.clone(),
                    };
                    (deref_expr, self.engine.resolve(inner.as_ref()))
                } else {
                    (self_var, resolved_self)
                };

                if let Type::Struct {
                    id,
                    args: ref type_args,
                } = base_ty
                {
                    if let Some(hir_struct) = self.items.structure(id) {
                        let substitution = Self::generic_substitution_for_owner(
                            hir_struct.id,
                            &hir_struct.generic_params,
                            type_args,
                        );
                        field_ty = hir_struct
                            .fields
                            .iter()
                            .find(|field| field.name == ident.name)
                            .map(|field| field.ty.substitute_generics(&substitution))
                            .unwrap_or(Type::Error);
                    }
                }
                let field = if let Type::Struct { id, .. } = base_ty {
                    self.items.structure(id).and_then(|hir_struct| {
                        hir_struct
                            .fields
                            .iter()
                            .find(|field| field.name == ident.name)
                            .map(|field| HirFieldLocation {
                                owner: hir_struct.id,
                                field_id: field.id,
                                name: ident.name.clone(),
                            })
                    })
                } else {
                    None
                };
                if let Some(field) = field.as_ref() {
                    self.record_source_reference(
                        ident.span.clone(),
                        crate::source_map::SourceSymbol::Field {
                            owner: field.owner,
                            field: field.field_id,
                        },
                    );
                }
                HirExpr {
                    ty: field_ty,
                    kind: HirExprKind::FieldAccess(Box::new(base_expr), ident.name.clone(), field),
                    span: ident.span.clone(),
                }
            }
            ast::Operand::Instance(inst) => self.lower_instance(inst),
            ast::Operand::LambdaDecl(lambda) => self.lower_lambda(lambda),
            ast::Operand::Tuple(tuple) => self.lower_tuple(tuple),
            ast::Operand::If(if_expr) => self.lower_if(if_expr),
            ast::Operand::Match(match_expr) => self.lower_match(match_expr),
            ast::Operand::Loop(loop_expr) => self.lower_loop(loop_expr),
            ast::Operand::Expression(expr) => self.lower_expression_with_use(expr, use_kind),
            ast::Operand::Unsafe(block, span) => {
                let body = self.with_unsafe(|s| s.lower_block(block));
                let ty = body.ty.clone();
                HirExpr {
                    ty,
                    kind: HirExprKind::UnsafeBlock(body),
                    span: span.clone(),
                }
            }
            ast::Operand::MacroInvoc(invocation) => {
                // Expansion replaces every macro call it can resolve, so one
                // reaching lowering names an unknown macro.
                self.diagnostics.push_type_with_span(
                    format!("Unknown macro '%{}'", invocation.name.name),
                    invocation.name.span.clone(),
                );
                self.error_expression_at(invocation.name.span.clone())
            }
            ast::Operand::NativeOperator(op) => {
                let ty = if crate::lower::intrinsics::is_intrinsic_name(&op.name) {
                    self.engine.fresh_type_var_at(op.span.clone())
                } else {
                    self.diagnostics.push_type_with_span(
                        format!("Unknown intrinsic '~{}'", op.name),
                        op.span.clone(),
                    );
                    Type::Error
                };
                HirExpr {
                    ty,
                    kind: HirExprKind::Var(op.name.clone()),
                    span: op.span.clone(),
                }
            }
        }
    }
}

//! Literal lowering.

use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::{ArrayLen, Type};

impl Lowerer {
    pub(crate) fn lower_literal(&mut self, lit: &ast::Literal) -> HirExpr {
        let span = lit.span.clone();
        match &lit.kind {
            ast::LiteralKind::Bool(b) => HirExpr {
                ty: Type::Bool,
                kind: HirExprKind::BoolLiteral(*b),
                span,
            },
            ast::LiteralKind::Number(n) => HirExpr {
                ty: Type::I64,
                kind: HirExprKind::IntLiteral(*n as i64),
                span,
            },
            ast::LiteralKind::Float(f) => HirExpr {
                ty: Type::F64,
                kind: HirExprKind::FloatLiteral(*f),
                span,
            },
            ast::LiteralKind::String(s) => HirExpr {
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                },
                kind: HirExprKind::StringLiteral(s.clone()),
                span,
            },
            ast::LiteralKind::Char(c) => {
                let value = match c.as_str() {
                    "\\n" => Some('\n'),
                    "\\t" => Some('\t'),
                    "\\r" => Some('\r'),
                    "\\0" => Some('\0'),
                    "\\\\" => Some('\\'),
                    "\\'" => Some('\''),
                    "\\\"" => Some('"'),
                    _ => {
                        let mut chars = c.chars();
                        chars.next().filter(|_| chars.next().is_none())
                    }
                };
                let Some(value) = value else {
                    self.diagnostics.push_selection_with_span(
                        "character literal must contain exactly one Unicode scalar or supported escape".to_string(),
                        span.clone(),
                    );
                    return self.error_expression_at(span);
                };
                HirExpr {
                    ty: Type::Char,
                    kind: HirExprKind::CharLiteral(value),
                    span,
                }
            }
            ast::LiteralKind::Array(arr) => {
                let elem_ty = self.engine.fresh_type_var_at(span.clone());
                let len = arr.elements.len();
                let elements: Vec<HirExpr> = arr
                    .elements
                    .iter()
                    .map(|e| {
                        let hir_e = self.lower_expression(e);
                        let resolved_elem_ty = self.engine.resolve(&elem_ty);
                        let resolved_hir_ty = self.engine.resolve(&hir_e.ty);
                        if !matches!(hir_e.kind, HirExprKind::Cast(_, _))
                            && resolved_hir_ty == Type::I64
                            && matches!(&resolved_elem_ty, Type::TypeVar(_))
                        {
                            if let Type::TypeVar(id) = resolved_elem_ty {
                                self.constraint_store
                                    .add_int_literal(id, hir_e.span.clone());
                            }
                        } else {
                            let _ = self.engine.unify(&elem_ty, &hir_e.ty);
                        }
                        hir_e
                    })
                    .collect();
                HirExpr {
                    ty: Type::Array(Box::new(elem_ty), ArrayLen::Value(len as u64)),
                    kind: HirExprKind::ArrayLiteral(elements),
                    span,
                }
            }
            ast::LiteralKind::ArrayRepeat { value, len } => {
                let value = self.lower_expression(value);
                let elem_ty = self.engine.fresh_type_var_at(value.span.clone());
                let resolved_elem_ty = self.engine.resolve(&elem_ty);
                let resolved_value_ty = self.engine.resolve(&value.ty);
                if !matches!(value.kind, HirExprKind::Cast(_, _))
                    && resolved_value_ty == Type::I64
                    && matches!(&resolved_elem_ty, Type::TypeVar(_))
                {
                    if let Type::TypeVar(id) = resolved_elem_ty {
                        self.constraint_store
                            .add_int_literal(id, value.span.clone());
                    }
                } else {
                    let _ = self.engine.unify(&elem_ty, &value.ty);
                }
                HirExpr {
                    ty: Type::Array(Box::new(elem_ty), ArrayLen::Value(*len as u64)),
                    kind: HirExprKind::ArrayRepeat(Box::new(value), *len),
                    span,
                }
            }
        }
    }

    pub(super) fn is_fat_raw_slice_pointer(ty: &Type) -> bool {
        matches!(ty, Type::Pointer(inner) if matches!(inner.as_ref(), Type::Slice(_)))
    }
}

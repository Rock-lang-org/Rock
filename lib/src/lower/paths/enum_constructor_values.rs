//! Positional enum constructors used as function values.

use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirParam, HirStmt, HirVarRef, HirVarTarget, HirVariantFields,
};
use crate::lower::Lowerer;
use crate::types::{FunctionSafety, Type};

impl Lowerer {
    pub(crate) fn enum_constructor_value(&mut self, mut expr: HirExpr) -> HirExpr {
        let HirExprKind::EnumVariant(_, _, args, Some(location)) = &expr.kind else {
            return expr;
        };
        if !args.is_empty() {
            return expr;
        }
        let Type::Enum { args: type_args, .. } = &expr.ty else {
            return expr;
        };
        let Some(enumeration) = self.items.enumeration(location.owner) else {
            return expr;
        };
        let Some(variant) = enumeration.variants.iter().find(|variant| variant.id == location.variant_id)
        else {
            return expr;
        };
        let HirVariantFields::Positional(fields) = &variant.fields else {
            return expr;
        };
        if fields.is_empty() {
            return expr;
        }
        let substitution = Self::generic_substitution_for_owner(
            enumeration.id,
            &enumeration.generic_params,
            type_args,
        );
        let param_types = fields
            .iter()
            .map(|ty| ty.substitute_generics(&substitution))
            .collect::<Vec<_>>();
        let params = param_types
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("<variant-arg-{index}>"),
                local_id: self.fresh_local_id(),
                ty: ty.clone(),
                mutable: false,
                is_ref: false,
            })
            .collect::<Vec<_>>();
        let span = expr.span.clone();
        if let HirExprKind::EnumVariant(_, _, args, _) = &mut expr.kind {
            *args = params
                .iter()
                .map(|param| HirExpr {
                    ty: param.ty.clone(),
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: param.name.clone(),
                        target: HirVarTarget::Local(param.local_id),
                    }),
                    span: span.clone(),
                })
                .collect();
        }
        let ret = expr.ty.clone();
        HirExpr {
            ty: Self::lambda_function_type(param_types, ret.clone(), FunctionSafety::Safe, &[]),
            kind: HirExprKind::Lambda {
                params,
                body: HirBlock {
                    ty: ret,
                    stmts: vec![HirStmt::Expr(expr)],
                },
                captures: Vec::new(),
            },
            span,
        }
    }
}

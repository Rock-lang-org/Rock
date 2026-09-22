use super::hir_types::{
    HirBlock, HirExpr, HirExprKind, HirMethodCallTarget, HirStmt, HirVarRef, HirVarTarget,
};
use super::Monomorphizer;
use crate::hir::HirParam;
use crate::ids::HirLocalId;
use crate::lexer::Span;
use crate::types::{CallableKind, GenericParamId, Type};

impl Monomorphizer {
    pub(super) fn infer_native_callable_bound_arguments(
        &mut self,
        function: &super::hir_types::HirFunction,
        generic_ids: &[GenericParamId],
        substitution: &mut std::collections::HashMap<GenericParamId, crate::ids::TypeId>,
    ) {
        for (subject, bounds) in &function.generic_bounds {
            if !substitution.contains_key(subject) {
                continue;
            }
            let actual = self.apply_substitution(&Type::Generic(*subject), substitution);
            let mut native = &actual;
            while let Type::Reference { inner, .. } = native {
                native = inner;
            }
            let Type::Function { params, ret, .. } = native else {
                continue;
            };
            let args = match params.as_slice() {
                [] => Type::Unit,
                [ty] => ty.clone(),
                params => Type::Tuple(params.to_vec()),
            };
            for bound in bounds {
                if bound.type_args.len() != 2
                    || !self
                        .language_items
                        .callable_protocols()
                        .any(|(_, id, _)| id == bound.trait_id)
                {
                    continue;
                }
                self.extract_generics_from_type(
                    &bound.type_args[0],
                    &args,
                    generic_ids,
                    substitution,
                );
                self.extract_generics_from_type(
                    &bound.type_args[1],
                    ret.as_ref(),
                    generic_ids,
                    substitution,
                );
            }
        }
    }

    /// Functions have a native ABI rather than a user impl body. A typed adapter
    /// evaluates the receiver and argument pack once and preserves ownership.
    pub(super) fn native_callable_adapter(
        &self,
        receiver: &HirExpr,
        args: &[HirExpr],
        target: &HirMethodCallTarget,
        result: &Type,
        span: &Span,
    ) -> Option<HirExpr> {
        let (kind, _, _) =
            self.language_items
                .callable_protocols()
                .find(|(_, trait_id, member_id)| {
                    target.trait_id() == Some(*trait_id) && target.method_id() == Some(*member_id)
                })?;
        let mut native_ty = &receiver.ty;
        while let Type::Reference { inner, .. } = native_ty {
            native_ty = inner;
        }
        let Type::Function {
            params: native_params,
            ..
        } = native_ty
        else {
            return None;
        };
        let [packed] = args else { return None };
        let mut receiver = receiver.clone();
        if kind != CallableKind::FnOnce {
            let mutable = kind == CallableKind::FnMut;
            receiver = HirExpr {
                ty: Type::Reference {
                    mutable,
                    inner: Box::new(receiver.ty.clone()),
                },
                kind: HirExprKind::Ref(mutable, Box::new(receiver)),
                span: span.clone(),
            };
        }
        let params = vec![
            HirParam {
                name: "__call_receiver".into(),
                local_id: HirLocalId(0),
                ty: receiver.ty.clone(),
                mutable: true,
                is_ref: false,
            },
            HirParam {
                name: "__call_args".into(),
                local_id: HirLocalId(1),
                ty: packed.ty.clone(),
                mutable: false,
                is_ref: false,
            },
        ];
        let parameter = |param: &HirParam| HirExpr {
            ty: param.ty.clone(),
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: param.name.clone(),
                target: HirVarTarget::Local(param.local_id),
            }),
            span: span.clone(),
        };
        let callee = parameter(&params[0]);
        let arguments = match native_params.as_slice() {
            [] => Vec::new(),
            [_] => vec![parameter(&params[1])],
            native_params => native_params
                .iter()
                .enumerate()
                .map(|(index, ty)| HirExpr {
                    ty: ty.clone(),
                    kind: HirExprKind::TupleIndex(Box::new(parameter(&params[1])), index as u32),
                    span: span.clone(),
                })
                .collect(),
        };
        let call = HirExpr {
            ty: result.clone(),
            kind: HirExprKind::Call(Box::new(callee), arguments, None),
            span: span.clone(),
        };
        let adapter = HirExpr {
            ty: Type::function(
                params.iter().map(|p| p.ty.clone()).collect(),
                result.clone(),
            ),
            kind: HirExprKind::Lambda {
                params,
                captures: Vec::new(),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(call)],
                    ty: result.clone(),
                },
            },
            span: span.clone(),
        };
        Some(HirExpr {
            ty: result.clone(),
            kind: HirExprKind::Call(Box::new(adapter), vec![receiver, packed.clone()], None),
            span: span.clone(),
        })
    }
}

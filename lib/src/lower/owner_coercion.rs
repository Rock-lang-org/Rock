use crate::hir::{HirExpr, HirExprKind, HirObjectCoercion};
use crate::types::Type;

use super::Lowerer;

impl Lowerer {
    pub(crate) fn lower_owned_object_method_value(
        &mut self,
        mut owner: HirExpr,
        name: &str,
        span: crate::lexer::Span,
    ) -> Option<HirExpr> {
        use crate::hir::*;
        owner.ty = self.engine.resolve(&owner.ty);
        let (selected, mut call) = match crate::hir::owned_objects::select(
            &owner,
            name,
            self.items.traits_for_selection(),
            &self.language_items,
            &self.selection_service(),
            self.opened_witnesses
                .iter()
                .rev()
                .find_map(|(_, binding)| binding.matches_owner(&owner.ty).then_some(binding)),
        ) {
            Ok(Some(selected)) => selected,
            Ok(None) => return None,
            Err(message) => {
                self.diagnostics
                    .push_type_with_span(message, owner.span.clone());
                return Some(self.error_expression_at(owner.span));
            }
        };
        let function = selected.function.as_ref()?;
        if function.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_type_with_span(
                "unsafe consuming method value requires an unsafe block".into(),
                span.clone(),
            );
            return Some(self.error_expression_at(span));
        }
        let substitution = self.fresh_method_generic_subst(function, span.clone());
        let params = selected
            .substituted_params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let mut parameter = parameter.clone();
                parameter.ty = parameter.ty.substitute_generics(&substitution);
                parameter.name = format!("<owned-arg-{index}>");
                parameter.local_id = self.fresh_local_id();
                parameter
            })
            .collect::<Vec<_>>();
        let arguments = params
            .iter()
            .map(|parameter| HirExpr {
                ty: parameter.ty.clone(),
                span: span.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: parameter.name.clone(),
                    target: HirVarTarget::Local(parameter.local_id),
                }),
            })
            .collect();
        let Some((function, result, args, method)) =
            self.selected_method_call_types(&selected, arguments)
        else {
            return Some(self.error_expression_at(span));
        };
        call.method = method;
        if let Some(member) = call.method.method_id() {
            self.record_source_reference(
                span.clone(),
                crate::source_map::SourceSymbol::Definition(member),
            );
        }
        // Evaluate the owning receiver once when binding the method value, not
        // on each invocation (especially important for calls/raw dereferences).
        let local_id = self.fresh_local_id();
        let owner_ty = owner.ty.clone();
        let owner_name = "<owned-method-self>".to_string();
        let captured_owner = HirExpr {
            ty: owner_ty.clone(),
            span: span.clone(),
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: owner_name.clone(),
                target: HirVarTarget::Local(local_id),
            }),
        };
        let body = HirBlock {
            ty: result.clone(),
            stmts: vec![HirStmt::Expr(HirExpr {
                ty: result.clone(),
                span: span.clone(),
                kind: HirExprKind::OwnedObjectCall {
                    owner: Box::new(captured_owner),
                    args,
                    call,
                },
            })],
        };
        let captures = vec![HirClosureCapture {
            name: owner_name.clone(),
            local_id,
            kind: HirClosureCaptureKind::Move,
            mutable: false,
            ty: owner_ty.clone(),
        }];
        let ty = Self::lambda_function_type(
            params
                .iter()
                .map(|parameter| parameter.ty.clone())
                .collect(),
            result,
            crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
            &captures,
        );
        let lambda = HirExpr {
            ty: ty.clone(),
            span: span.clone(),
            kind: HirExprKind::Lambda {
                params,
                body,
                captures,
            },
        };
        Some(HirExpr {
            ty: ty.clone(),
            span,
            kind: HirExprKind::Block(HirBlock {
                ty,
                stmts: vec![
                    HirStmt::Let {
                        name: owner_name,
                        local_id,
                        ty: owner_ty,
                        value: owner,
                        mutable: false,
                    },
                    HirStmt::Expr(lambda),
                ],
            }),
        })
    }

    pub(crate) fn lower_owned_object_call(
        &mut self,
        mut owner: HirExpr,
        name: &str,
        arguments: &[crate::ast::Argument],
        span: crate::lexer::Span,
    ) -> Option<HirExpr> {
        owner.ty = self.engine.resolve(&owner.ty);
        let selection = crate::hir::owned_objects::select(
            &owner,
            name,
            self.items.traits_for_selection(),
            &self.language_items,
            &self.selection_service(),
            self.opened_witnesses
                .iter()
                .rev()
                .find_map(|(_, binding)| binding.matches_owner(&owner.ty).then_some(binding)),
        );
        let (selected, mut call) = match selection {
            Ok(Some(selected)) => selected,
            Ok(None) => return None,
            Err(message) => {
                self.diagnostics
                    .push_type_with_span(message, owner.span.clone());
                return Some(self.error_expression_at(owner.span));
            }
        };
        let args = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.lower_selected_method_argument(&argument.arg, &selected, index)
            })
            .collect::<Vec<_>>();
        if args.len() != selected.substituted_params.len() {
            self.diagnostics.push_type_with_span(
                "consuming object call requires all declared arguments".into(),
                owner.span.clone(),
            );
            return Some(self.error_expression_at(owner.span));
        }
        let Some((function, ty, args, method)) = self.selected_method_call_types(&selected, args)
        else {
            return Some(self.error_expression_at(owner.span));
        };
        if function.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_type_with_span(
                "unsafe consuming object method requires an unsafe block".into(),
                owner.span.clone(),
            );
            return Some(self.error_expression_at(owner.span));
        }
        call.method = method;
        if let Some(member) = call.method.method_id() {
            self.record_source_reference(
                span.clone(),
                crate::source_map::SourceSymbol::Definition(member),
            );
        }
        Some(HirExpr {
            span,
            ty,
            kind: HirExprKind::OwnedObjectCall {
                owner: Box::new(owner),
                args,
                call,
            },
        })
    }

    pub(crate) fn coerce_object_owner(&mut self, value: HirExpr, target: &Type) -> Option<HirExpr> {
        if !crate::infer::owner_coercion::is_owner_destination(target)
            || self.engine.resolve(&value.ty) == *target
        {
            return None;
        }
        // Do not equate the source payload with the unsized destination before
        // its producer/SCC has finished inference.
        Some(HirExpr {
            span: value.span.clone(),
            ty: target.clone(),
            kind: HirExprKind::ObjectCoercion(
                Box::new(value),
                HirObjectCoercion {
                    target: target.clone(),
                    evidence: None,
                },
            ),
        })
    }
}

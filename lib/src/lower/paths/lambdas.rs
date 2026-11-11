//! Lambda lowering.

use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety, Type};

impl Lowerer {
    pub(crate) fn lambda_function_type(
        params: Vec<Type>,
        ret: Type,
        safety: FunctionSafety,
        captures: &[HirClosureCapture],
    ) -> Type {
        let captures = captures
            .iter()
            .map(|capture| FunctionCapture {
                effect: match capture.kind {
                    HirClosureCaptureKind::SharedBorrow => crate::types::CaptureEffect::Read,
                    HirClosureCaptureKind::MutableBorrow => crate::types::CaptureEffect::Mutate,
                    HirClosureCaptureKind::Move => crate::types::CaptureEffect::Consume,
                },
                kind: match capture.kind {
                    HirClosureCaptureKind::SharedBorrow => CaptureKind::SharedBorrow,
                    HirClosureCaptureKind::MutableBorrow => CaptureKind::MutableBorrow,
                    HirClosureCaptureKind::Move => CaptureKind::Move,
                },
                ty: capture.ty.clone(),
            })
            .collect::<Vec<_>>();
        let callable_kind = CallableKind::from_captures(&captures);
        Type::function_with_metadata(params, ret, safety, callable_kind, captures)
    }

    fn apply_lambda_body_effects(ty: &mut Type, effects: &[HirClosureCapture]) {
        if let Type::Function { captures, callable_kind, .. } = ty {
            for (capture, effect) in captures.iter_mut().zip(effects) {
                capture.effect = match effect.kind {
                    HirClosureCaptureKind::SharedBorrow => crate::types::CaptureEffect::Read,
                    HirClosureCaptureKind::MutableBorrow => crate::types::CaptureEffect::Mutate,
                    HirClosureCaptureKind::Move => crate::types::CaptureEffect::Consume,
                };
            }
            *callable_kind = CallableKind::from_captures(captures);
        }
    }

    fn choose_capture_storage(&self, captures: &mut [HirClosureCapture], lambda: Option<&ast::LambdaDecl>) {
        let live = lambda.and_then(|lambda| self.body.capture_live_after.get(&(lambda as *const _ as usize)));
        for capture in captures {
            let ty = self.engine.resolve(&capture.ty);
            if crate::type_services::facts::TypeFacts::is_copy(&ty)
                && capture.kind != HirClosureCaptureKind::MutableBorrow
            {
                capture.kind = HirClosureCaptureKind::Move;
            } else if live.is_some_and(|live| live.contains(&capture.name))
                || self.body.borrowed_capture_sources.contains(&capture.local_id)
            {
                capture.kind = if capture.kind == HirClosureCaptureKind::MutableBorrow {
                    HirClosureCaptureKind::MutableBorrow
                } else {
                    HirClosureCaptureKind::SharedBorrow
                };
            } else {
                capture.kind = HirClosureCaptureKind::Move;
            }
        }
    }

    pub(crate) fn lower_lambda(&mut self, lambda: &ast::LambdaDecl) -> HirExpr {
        self.lower_lambda_with_expected_params(lambda, None)
    }

    pub(crate) fn lower_lambda_with_expected_params(
        &mut self,
        lambda: &ast::LambdaDecl,
        expected_params: Option<&[Type]>,
    ) -> HirExpr {
        let enclosing_borrows = self.body.borrowed_capture_sources.clone();
        if let Some(live) = self.body.capture_live_after.get(&(lambda as *const _ as usize)) {
            for name in live {
                if let Some(binding) = self.scope.lookup(name) {
                    if !crate::type_services::facts::TypeFacts::is_copy(&self.engine.resolve(&binding.ty)) {
                        if let Some(id) = binding.local_id {
                            self.body.borrowed_capture_sources.insert(id);
                        }
                    }
                }
            }
        }
        if matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried) && lambda.parameters.len() > 1 {
            let mut value = self.lower_curried_lambda(lambda, &lambda.span);
            self.body.borrowed_capture_sources = enclosing_borrows;
            if let HirExprKind::Lambda { captures, .. } = &mut value.kind {
                self.choose_capture_storage(captures, Some(lambda));
                if let Type::Function { captures: metadata, .. } = &mut value.ty {
                    for (metadata, capture) in metadata.iter_mut().zip(captures) {
                        metadata.kind = match capture.kind {
                            HirClosureCaptureKind::Move => CaptureKind::Move,
                            HirClosureCaptureKind::SharedBorrow => CaptureKind::SharedBorrow,
                            HirClosureCaptureKind::MutableBorrow => CaptureKind::MutableBorrow,
                        };
                    }
                }
            }
            return value;
        }

        let mut params = Vec::new();
        let mut param_types = Vec::new();

        self.body.enter_callable_scope(self.scope.depth());
        self.push_scope();

        for (index, param_pat) in lambda.parameters.iter().enumerate() {
            let (name, inner_ty, mutable, is_ref) =
                self.lower_param_pattern_at(param_pat, lambda.span.clone());
            let inferred_ty = if is_ref {
                Type::Reference {
                    mutable,
                    inner: Box::new(inner_ty),
                }
            } else {
                inner_ty
            };
            let ty = expected_params
                .and_then(|params| params.get(index))
                .cloned()
                .unwrap_or(inferred_ty);
            let local_id = self.fresh_local_id();
            self.scope
                .define_local(name.clone(), ty.clone(), mutable, local_id);
            if let Some(owner) = self.current_body_def_id() {
                let span =
                    Self::pattern_binding_span(param_pat).unwrap_or_else(|| lambda.span.clone());
                self.source_map.insert_local_in_scope(
                    owner,
                    local_id,
                    span,
                    self.body.current_source_scope(),
                );
            }
            param_types.push(ty.clone());
            params.push(HirParam {
                name,
                local_id,
                ty,
                mutable,
                is_ref,
            });
        }

        let lambda_return_ty = self.engine.fresh_type_var_at(lambda.span.clone());
        let loop_stack = self.body.suspend_loops();
        let body = self.with_body_return_type(lambda_return_ty.clone(), |lowerer| {
            lowerer.lower_lambda_body(lambda)
        });
        self.body.resume_loops(loop_stack);
        let span = lambda.span.clone();
        let _ = self.engine.unify(&body.ty, &lambda_return_ty);
        let ret_type = self.engine.resolve(&lambda_return_ty);
        self.body.borrowed_capture_sources = enclosing_borrows;
        let mut captures = self.collect_lambda_captures(&body, &params);
        let body_effects = captures.clone();
        self.choose_capture_storage(&mut captures, Some(lambda));

        self.pop_scope();
        self.body.leave_callable_scope();

        let mut func_type =
            Self::lambda_function_type(param_types, ret_type, FunctionSafety::Safe, &captures);
        Self::apply_lambda_body_effects(&mut func_type, &body_effects);

        HirExpr {
            ty: func_type,
            kind: HirExprKind::Lambda {
                params,
                body,
                captures,
            },
            span,
        }
    }

    pub(crate) fn lower_curried_lambda(
        &mut self,
        lambda: &ast::LambdaDecl,
        span: &crate::lexer::Span,
    ) -> HirExpr {
        self.lower_curried_lambda_stage(&lambda.parameters, &lambda.body, span)
    }

    pub(crate) fn lower_curried_lambda_stage(
        &mut self,
        params: &[ast::Pattern],
        body: &ast::Block,
        span: &crate::lexer::Span,
    ) -> HirExpr {
        self.lower_curried_lambda_stage_with_expected_params(params, body, span, None)
    }

    pub(crate) fn lower_curried_lambda_stage_with_expected_params(
        &mut self,
        params: &[ast::Pattern],
        body: &ast::Block,
        span: &crate::lexer::Span,
        expected_params: Option<&[Type]>,
    ) -> HirExpr {
        self.body.enter_callable_scope(self.scope.depth());
        self.push_scope();

        let (name, inferred_ty, mutable, is_ref) =
            self.lower_param_pattern_at(&params[0], span.clone());
        let ty = expected_params
            .and_then(|params| params.first())
            .cloned()
            .unwrap_or(inferred_ty);
        let local_id = self.fresh_local_id();
        self.scope
            .define_local(name.clone(), ty.clone(), mutable, local_id);
        if let Some(owner) = self.current_body_def_id() {
            let param_span = Self::pattern_binding_span(&params[0]).unwrap_or_else(|| span.clone());
            self.source_map.insert_local_in_scope(
                owner,
                local_id,
                param_span,
                self.body.current_source_scope(),
            );
        }
        let param = HirParam {
            name,
            local_id,
            ty: ty.clone(),
            mutable,
            is_ref,
        };

        let lambda_return_ty = self.engine.fresh_type_var_at(span.clone());
        let loop_stack = self.body.suspend_loops();
        let body = if params.len() == 1 {
            self.with_body_return_type(lambda_return_ty.clone(), |lowerer| {
                lowerer.prepare_capture_liveness(body);
                lowerer.lower_block(body)
            })
        } else {
            let inner = self.lower_curried_lambda_stage_with_expected_params(
                &params[1..],
                body,
                span,
                expected_params.and_then(|params| params.get(1..)),
            );
            HirBlock {
                ty: inner.ty.clone(),
                stmts: vec![HirStmt::Expr(inner)],
            }
        };
        self.body.resume_loops(loop_stack);
        let _ = self.engine.unify(&body.ty, &lambda_return_ty);
        let body_ty = self.engine.resolve(&lambda_return_ty);
        let mut captures = self.collect_lambda_captures(&body, std::slice::from_ref(&param));
        let body_effects = captures.clone();
        // A synthetic curried stage only returns the next stage. It has no
        // later uses of its own, but cannot transfer ownership of a source
        // that its enclosing closure only borrowed.
        self.choose_capture_storage(&mut captures, None);

        self.pop_scope();
        self.body.leave_callable_scope();

        let mut func_type = Self::lambda_function_type(vec![ty], body_ty, FunctionSafety::Safe, &captures);
        Self::apply_lambda_body_effects(&mut func_type, &body_effects);
        HirExpr {
            ty: func_type,
            kind: HirExprKind::Lambda {
                params: vec![param],
                body,
                captures,
            },
            span: span.clone(),
        }
    }
}

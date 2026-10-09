use crate::ast;
use crate::hir::*;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use crate::lower::statement::BlockResult;
use crate::lower::Lowerer;

impl Lowerer {
    pub(crate) fn lower_if(&mut self, if_expr: &ast::If) -> HirExpr {
        self.lower_if_expected(if_expr, None)
    }

    pub(crate) fn lower_if_expected(
        &mut self,
        if_expr: &ast::If,
        expected: Option<&Type>,
    ) -> HirExpr {
        self.lower_if_with_result(if_expr, expected, BlockResult::Value)
    }

    pub(crate) fn lower_if_discarded(&mut self, if_expr: &ast::If) -> HirExpr {
        self.lower_if_with_result(if_expr, None, BlockResult::Discarded)
    }

    fn lower_if_with_result(
        &mut self,
        if_expr: &ast::If,
        expected: Option<&Type>,
        result: BlockResult,
    ) -> HirExpr {
        let condition = self.lower_expression(&if_expr.condition.expression);
        let span = condition.span.clone();
        let _ = self.engine.unify(&condition.ty, &Type::Bool);

        // An inferred result slot belongs to the join, not its first branch.
        // Keep concrete expectations so declared signatures still guide coercions.
        let branch_expected =
            expected.filter(|ty| !matches!(self.engine.resolve(ty), Type::TypeVar(_)));
        self.push_scope();
        let mut then_branch = match result {
            BlockResult::Value => self.lower_block_expected(&if_expr.then, branch_expected),
            BlockResult::Discarded => self.lower_block_discarded(&if_expr.then, &span),
        };
        if result == BlockResult::Value {
            self.finish_control_flow_branch(&mut then_branch, &span);
        }
        self.pop_scope();

        let else_branch = if_expr.else_.as_ref().map(|else_| match else_ {
            ast::Else::Block(block) => {
                self.push_scope();
                let mut b = match result {
                    BlockResult::Value => self.lower_block_expected(block, branch_expected),
                    BlockResult::Discarded => self.lower_block_discarded(block, &span),
                };
                if result == BlockResult::Value {
                    self.finish_control_flow_branch(&mut b, &span);
                }
                self.pop_scope();
                b
            }
            ast::Else::If(nested_if) => {
                let nested = self.lower_if_with_result(nested_if, branch_expected, result);
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

        let result = HirExpr {
            ty: result_ty,
            kind: HirExprKind::If {
                condition: Box::new(condition),
                then_branch,
                else_branch,
            },
            span,
        };
        match (expected, branch_expected) {
            (Some(expected), None) => self.coerce_argument_to_expected(result, expected),
            _ => result,
        }
    }

    pub(crate) fn lower_match(&mut self, match_expr: &ast::Match) -> HirExpr {
        self.lower_match_expected(match_expr, None)
    }

    pub(crate) fn lower_match_expected(
        &mut self,
        match_expr: &ast::Match,
        expected: Option<&Type>,
    ) -> HirExpr {
        self.lower_match_with_result(match_expr, expected, BlockResult::Value)
    }

    pub(crate) fn lower_match_discarded(&mut self, match_expr: &ast::Match) -> HirExpr {
        self.lower_match_with_result(match_expr, None, BlockResult::Discarded)
    }

    fn lower_match_with_result(
        &mut self,
        match_expr: &ast::Match,
        expected: Option<&Type>,
        result: BlockResult,
    ) -> HirExpr {
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
        let branch_expected =
            expected.filter(|ty| !matches!(self.engine.resolve(ty), Type::TypeVar(_)));

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

                let mut body = match result {
                    BlockResult::Value => self.lower_block_expected(&arm.body, branch_expected),
                    BlockResult::Discarded => self.lower_block_discarded(&arm.body, &span),
                };
                if result == BlockResult::Value {
                    self.finish_control_flow_branch(&mut body, &span);
                }
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

        let result = HirExpr {
            ty: result_ty
                .map(|ty| self.engine.resolve(&ty))
                .unwrap_or(Type::Unit),
            kind: HirExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        };
        match (expected, branch_expected) {
            (Some(expected), None) => self.coerce_argument_to_expected(result, expected),
            _ => result,
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

    pub(crate) fn finish_control_flow_branch(
        &mut self,
        block: &mut HirBlock,
        span: &crate::lexer::Span,
    ) {
        // A bare return supplies unit to its owning function, not to this join.
        // Value returns already receive their source return expectation in lower_statement.
        for stmt in &mut block.stmts {
            match stmt {
                HirStmt::Return(None) => {
                    if let Some(expected) = self.source_return_type() {
                        if let Err(error) = self.engine.unify(&Type::Unit, &expected) {
                            self.diagnostics
                                .push_type_with_span(error.render(&self.engine), span.clone());
                        }
                    }
                }
                HirStmt::Expr(expr) | HirStmt::Let { value: expr, .. } => match &mut expr.kind {
                    HirExprKind::Block(inner)
                    | HirExprKind::UnsafeBlock(inner)
                    | HirExprKind::Open { body: inner, .. }
                    | HirExprKind::PackageOpen { body: inner, .. } => {
                        self.finish_control_flow_branch(inner, span);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if !Self::branch_completes_normally(block) {
            block.ty = Type::Never;
        }
    }

    fn branch_completes_normally(block: &HirBlock) -> bool {
        block.stmts.iter().all(|stmt| match stmt {
            HirStmt::Return(_) | HirStmt::Break(_) | HirStmt::Continue => false,
            HirStmt::Expr(expr) | HirStmt::Let { value: expr, .. } => {
                if matches!(expr.ty, Type::Never) {
                    return false;
                }
                match &expr.kind {
                    HirExprKind::Block(inner)
                    | HirExprKind::UnsafeBlock(inner)
                    | HirExprKind::Open { body: inner, .. }
                    | HirExprKind::PackageOpen { body: inner, .. } => {
                        Self::branch_completes_normally(inner)
                    }
                    _ => true,
                }
            }
        })
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
            // Diverging branches have no value at the join and must not bind
            // or conservatively weaken the remaining branch's callable slots.
            (Type::Never, _) => right,
            (_, Type::Never) => left,
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
                    result_borrowing: left_borrowing,
                },
                Type::Function {
                    params: right_params,
                    ret: right_ret,
                    safety: right_safety,
                    callable_kind: right_kind,
                    captures: right_captures,
                    result_borrowing: right_borrowing,
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
                let mut captures = left_captures.to_vec();
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
                    captures: captures.into_boxed_slice(),
                    result_borrowing: left_borrowing.merge(right_borrowing, covariant),
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
                        let left = self.display_type(&left);
                        let right = self.display_type(&right);
                        self.diagnostics.push_type_with_span(
                            format!("Type mismatch: {} vs {}", left, right),
                            span.clone(),
                        );
                        Type::Error
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
                result_borrowing,
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
                result_borrowing: if covariant {
                    crate::types::CallableResultBorrowing::Unknown
                } else {
                    result_borrowing.clone()
                },
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    use crate::crate_system::CrateContext;
    use crate::infer::ResolvedHirProgram;
    use crate::lower::ResolveError;
    use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety};
    use crate::{collect, infer, lower, parser, Config};

    static SOURCE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn resolved_source(source: &str) -> Result<ResolvedHirProgram, Vec<ResolveError>> {
        let id = SOURCE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("rock_callable_join_{}_{}", std::process::id(), id,));
        let entry_file = dir.join("main.rk");
        let config = Config {
            entry_file: entry_file.clone(),
            output_dir: dir,
            debug_print: Vec::new(),
            meta_files: Vec::new(),
            extern_artifacts: Vec::new(),
            source_providers: Vec::new(),
            current_crate_name: Some("test".to_string()),
            opt_level: 0,
            emit_llvm: false,
            no_link: true,
            emit_object: None,
            no_prelude: true,
            no_std: true,
            sysroot: None,
        };
        let ast = crate::ast::Program {
            module: parser::parse_source(entry_file, source, &config).expect("test source parses"),
        };
        let crate_ctx = CrateContext::new();
        let decls = collect::collect(&ast, &crate_ctx, false, Some("test"))
            .expect("test declarations collect");
        let partial =
            lower::program::lower_from_declarations(&ast, decls, &crate_ctx, Some("test"))?;
        infer::finalize(partial)
    }

    fn callable_source(control_flow: &str, projection: &str, call: &str) -> String {
        format!(
            r#"unsafe unchecked_reader: I64
unchecked_reader = -> 7

safe_reader: I64
safe_reader = -> 0

make_value = flag ->
{control_flow}

main = !->
    result = make_value true
    reader = {projection}
    value = {call}
"#
        )
    }

    #[test]
    fn inferred_callable_joins_preserve_unsafe_calls_in_both_orders() {
        for match_ in [false, true] {
            for reverse in [false, true] {
                for depth in 0..=2 {
                    let mut safe = "safe_reader".to_string();
                    let mut unsafe_ = "unsafe unchecked_reader".to_string();
                    let mut projection = "result".to_string();
                    for _ in 0..depth {
                        safe = format!("({safe}, true)");
                        unsafe_ = format!("({unsafe_}, true)");
                        projection = format!("({projection}).0");
                    }
                    let (first, second) = if reverse {
                        (&unsafe_, &safe)
                    } else {
                        (&safe, &unsafe_)
                    };
                    let control_flow = if match_ {
                        format!(
                            "    match flag\n        true => {first}\n        false => {second}"
                        )
                    } else {
                        format!("    if flag\n        {first}\n    else\n        {second}")
                    };
                    let source = callable_source(&control_flow, &projection, "unsafe reader!");
                    let resolved =
                        resolved_source(&source).expect("unsafe call accepts joined value");
                    let (_, make_value) = resolved
                        .program
                        .test_function_by_name("make_value")
                        .unwrap();
                    let mut ty = &make_value.ret_type;
                    for _ in 0..depth {
                        let Type::Tuple(fields) = ty else {
                            panic!("expected tuple result, got {ty:?}");
                        };
                        ty = &fields[0];
                    }
                    assert!(matches!(
                        ty,
                        Type::Function {
                            safety: FunctionSafety::Unsafe,
                            ..
                        }
                    ));

                    let source = callable_source(&control_flow, &projection, "reader!");
                    let errors =
                        resolved_source(&source).expect_err("joined callable requires unsafe");
                    assert!(
                        errors
                            .iter()
                            .any(|error| error.message.contains("requires an unsafe block")),
                        "{errors:?}"
                    );
                    assert!(
                        !errors.iter().any(|error| error
                            .message
                            .contains("Unsafe function cannot be used as safe function")),
                        "{errors:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn all_safe_callable_joins_remain_safe() {
        for control_flow in [
            "    if flag\n        safe_reader\n    else\n        safe_reader",
            "    match flag\n        true => safe_reader\n        false => safe_reader",
        ] {
            let source = callable_source(control_flow, "result", "reader!");
            let resolved = resolved_source(&source).expect("safe join accepts safe call");
            let (_, make_value) = resolved
                .program
                .test_function_by_name("make_value")
                .unwrap();
            assert!(matches!(
                make_value.ret_type,
                Type::Function {
                    safety: FunctionSafety::Safe,
                    ..
                }
            ));
        }
    }

    #[test]
    fn declared_safe_callable_result_rejects_unsafe_branches_in_both_orders() {
        for control_flow in [
            "    if flag\n        safe_reader\n    else\n        unsafe unchecked_reader",
            "    if flag\n        unsafe unchecked_reader\n    else\n        safe_reader",
            "    match flag\n        true => safe_reader\n        false => unsafe unchecked_reader",
            "    match flag\n        true => unsafe unchecked_reader\n        false => safe_reader",
        ] {
            let source = callable_source(control_flow, "result", "unsafe reader!").replace(
                "make_value = flag ->",
                "make_value: Bool -> (() -> I64)\nmake_value = flag ->",
            );
            let errors =
                resolved_source(&source).expect_err("unsafe branch violates safe signature");
            assert!(
                errors.iter().any(|error| error
                    .message
                    .contains("Unsafe function cannot be used as safe function")),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn callable_join_respects_parameter_variance_and_metadata() {
        let capture = FunctionCapture::new(
            CaptureKind::Move,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
        );
        let safe = Type::function(
            vec![Type::function(Vec::new(), Type::I64)],
            Type::Tuple(vec![Type::function(Vec::new(), Type::I64)]),
        );
        let unsafe_ = Type::function_with_metadata(
            vec![Type::unsafe_function(Vec::new(), Type::I64)],
            Type::Tuple(vec![Type::unsafe_function(Vec::new(), Type::I64)]),
            FunctionSafety::Unsafe,
            CallableKind::FnOnce,
            vec![capture.clone()],
        );
        let expected = Type::function_with_metadata(
            vec![Type::function(Vec::new(), Type::I64)],
            Type::Tuple(vec![Type::unsafe_function(Vec::new(), Type::I64)]),
            FunctionSafety::Unsafe,
            CallableKind::FnOnce,
            vec![capture],
        );
        for (left, right) in [(&safe, &unsafe_), (&unsafe_, &safe)] {
            let mut lowerer = Lowerer::with_options(false);
            let joined = lowerer.merge_control_flow_types(left, right, &crate::lexer::Span::test());
            assert_eq!(joined, expected);
            assert!(!lowerer.has_errors());
            assert!(lowerer.engine.unify(left, &joined).is_ok());
            assert!(lowerer.engine.unify(right, &joined).is_ok());
        }
    }

    #[test]
    fn callable_join_rejects_incompatible_signatures_and_invariant_storage() {
        let safe = Type::function(Vec::new(), Type::I64);
        let unsafe_ = Type::unsafe_function(Vec::new(), Type::I64);
        let mismatches = [
            (safe.clone(), Type::function(vec![Type::Bool], Type::I64)),
            (safe.clone(), Type::function(Vec::new(), Type::Bool)),
            (
                Type::function(vec![Type::I64], Type::I64),
                Type::function(vec![Type::Bool], Type::I64),
            ),
            (
                Type::Pointer(Box::new(safe.clone())),
                Type::Pointer(Box::new(unsafe_.clone())),
            ),
            (
                Type::Reference {
                    mutable: true,
                    inner: Box::new(safe),
                },
                Type::Reference {
                    mutable: true,
                    inner: Box::new(unsafe_),
                },
            ),
        ];
        for (left, right) in mismatches {
            for (left, right) in [(&left, &right), (&right, &left)] {
                let mut lowerer = Lowerer::with_options(false);
                lowerer.merge_control_flow_types(left, right, &crate::lexer::Span::test());
                assert!(lowerer.has_errors());
            }
        }
    }

    #[test]
    fn return_branches_join_only_their_fallthrough_values() {
        for match_ in [false, true] {
            for reverse in [false, true] {
                let (first, second) = if reverse {
                    ("()", "return failure")
                } else {
                    ("return failure", "()")
                };
                let branch = if match_ {
                    format!("    match flag\n        true => {first}\n        false => {second}")
                } else {
                    format!("    if flag\n        {first}\n    else\n        {second}")
                };
                let source = format!(
                    r#"enum Result T, E
    Ok T
    Err E

choose: Bool -> Result (I64, I64), I64
choose = flag ->
    failure: Result (I64, I64), I64 = Result::Err 5
    success: Result (I64, I64), I64 = Result::Ok (7, 9)
{branch}
    success

main = !->
    result = choose true
"#
                );
                let resolved = resolved_source(&source).expect("return arm has no join value");
                let (_, choose) = resolved.program.test_function_by_name("choose").unwrap();
                let crate::hir::HirStmtFor::Expr(join) = &choose.body.stmts[2] else {
                    panic!("expected branch expression");
                };
                assert_eq!(join.ty, Type::Unit);
                let branches = match &join.kind {
                    crate::hir::HirExprKindFor::If {
                        then_branch,
                        else_branch,
                        ..
                    } => {
                        vec![then_branch, else_branch.as_ref().unwrap()]
                    }
                    crate::hir::HirExprKindFor::Match { arms, .. } => {
                        arms.iter().map(|arm| &arm.body).collect()
                    }
                    _ => panic!("expected if or match"),
                };
                assert_eq!(branches[usize::from(reverse)].ty, Type::Never);
                assert_eq!(branches[usize::from(!reverse)].ty, Type::Unit);
            }
        }
    }

    #[test]
    fn both_returning_branches_are_never_but_keep_the_source_return_type() {
        for branch in [
            "    if flag\n        return failure\n    else\n        return success",
            "    match flag\n        true => return failure\n        false => return success",
        ] {
            let source = format!(
                r#"enum Result T, E
    Ok T
    Err E

choose: Bool -> Result (I64, I64), I64
choose = flag ->
    failure: Result (I64, I64), I64 = Result::Err 5
    success: Result (I64, I64), I64 = Result::Ok (7, 9)
{branch}

main = !->
    result = choose true
"#
            );
            let resolved =
                resolved_source(&source).expect("both branches return the source result");
            let (_, choose) = resolved.program.test_function_by_name("choose").unwrap();
            assert!(matches!(choose.ret_type, Type::Enum { .. }));
            assert_eq!(choose.body.ty, Type::Never);
        }
        for branch in [
            "    if flag\n        return\n    else\n        return",
            "    match flag\n        true => return\n        false => return",
        ] {
            let source = format!("finish = flag ->\n{branch}\n\nmain = !->\n    finish true\n");
            let resolved = resolved_source(&source).expect("bare returns infer the owner's unit");
            let (_, finish) = resolved.program.test_function_by_name("finish").unwrap();
            assert_eq!(finish.ret_type, Type::Unit);
            assert_eq!(finish.body.ty, Type::Never);
        }
    }

    #[test]
    fn unit_return_break_and_continue_do_not_contribute_join_values() {
        for exit in ["return", "break", "continue"] {
            for match_ in [false, true] {
                let branch = if match_ {
                    format!("    match flag\n        true => {exit}\n        false => 7")
                } else {
                    format!("    if flag\n        {exit}\n    else\n        7")
                };
                let source = if exit == "return" {
                    format!("finish = flag !->\n{branch}\n\nmain = !->\n    finish true\n")
                } else {
                    let branch = branch.replace('\n', "\n    ");
                    format!("finish = flag !->\n    while flag\n    {branch}\n\nmain = !->\n    finish false\n")
                };
                resolved_source(&source).expect("abrupt unit branch joins with the integer value");
            }
        }
    }

    #[test]
    fn nondiverging_unit_mismatches_and_invalid_bare_returns_are_rejected() {
        for branch in [
            "    if flag\n        ()\n    else\n        7 as I64",
            "    if flag\n        7 as I64\n    else\n        ()",
            "    match flag\n        true => ()\n        false => 7 as I64",
            "    match flag\n        true => 7 as I64\n        false => ()",
        ] {
            let source = format!("choose = flag ->\n{branch}\n\nmain = !->\n    choose true\n");
            let errors = resolved_source(&source).expect_err("normal unit and integer cannot join");
            assert!(errors
                .iter()
                .any(|error| error.message.contains("Type mismatch")));
        }
        let errors = resolved_source(
            "choose: Bool -> I64\nchoose = flag ->\n    if flag\n        return\n    else\n        return\n\nmain = !->\n    choose true\n",
        ).expect_err("divergence does not exempt the owner's return signature");
        assert!(errors
            .iter()
            .any(|error| error.message.contains("Type mismatch")));
    }

    #[test]
    fn never_join_is_neutral_without_binding_or_weakening_callable_slots() {
        for reverse in [false, true] {
            let mut lowerer = Lowerer::with_options(false);
            let unknown = lowerer.engine.fresh_type_var();
            let safe = Type::function(Vec::new(), Type::I64);
            let never = Type::Never;
            for value in [&unknown, &safe, &never] {
                let (left, right) = if reverse {
                    (value, &never)
                } else {
                    (&never, value)
                };
                let joined =
                    lowerer.merge_control_flow_types(left, right, &crate::lexer::Span::test());
                assert_eq!(&joined, value);
                assert_eq!(lowerer.engine.resolve(&unknown), unknown);
                assert!(!lowerer.has_errors());
            }
        }
    }

    #[test]
    fn discarded_if_and_match_preserve_typed_values_before_real_unit_tails() {
        for match_ in [false, true] {
            for reverse in [false, true] {
                let (first, second) = if reverse {
                    ("7 as I32", "()")
                } else {
                    ("()", "7 as I32")
                };
                let branch = if match_ {
                    format!("    match flag\n        true => {first}\n        false => {second}")
                } else {
                    format!("    if flag\n        {first}\n    else\n        {second}")
                };
                for unit_arrow in [false, true] {
                    let declaration = if unit_arrow {
                        format!("effects = flag !->\n{branch}")
                    } else {
                        format!("effects: Bool -> I64\neffects = flag ->\n{branch}\n    9 as I64")
                    };
                    let source = format!("{declaration}\n\nmain = !->\n    effects true\n");
                    let resolved = resolved_source(&source)
                        .expect("effect position discards each branch result");
                    let (_, effects) = resolved.program.test_function_by_name("effects").unwrap();
                    assert_eq!(
                        effects.ret_type,
                        if unit_arrow { Type::Unit } else { Type::I64 }
                    );
                    let crate::hir::HirStmtFor::Expr(join) = &effects.body.stmts[0] else {
                        panic!("expected discarded branch expression");
                    };
                    assert_eq!(join.ty, Type::Unit);
                    let branches = match &join.kind {
                        crate::hir::HirExprKindFor::If {
                            then_branch,
                            else_branch,
                            ..
                        } => {
                            vec![then_branch, else_branch.as_ref().unwrap()]
                        }
                        crate::hir::HirExprKindFor::Match { arms, .. } => {
                            arms.iter().map(|arm| &arm.body).collect()
                        }
                        _ => panic!("expected if or match"),
                    };
                    for (index, body) in branches.iter().enumerate() {
                        assert_eq!(body.ty, Type::Unit);
                        let crate::hir::HirStmtFor::Expr(original) = &body.stmts[0] else {
                            panic!("expected original branch value");
                        };
                        assert_eq!(
                            original.ty,
                            if index == usize::from(reverse) {
                                Type::Unit
                            } else {
                                Type::I32
                            }
                        );
                        let crate::hir::HirStmtFor::Expr(unit) = body.stmts.last().unwrap() else {
                            panic!("expected semantic unit producer");
                        };
                        assert!(matches!(unit.kind, crate::hir::HirExprKindFor::Unit));
                        assert_eq!(unit.ty, Type::Unit);
                    }
                }
            }
        }
    }

    #[test]
    fn discarded_context_propagates_into_nested_control_flow() {
        resolved_source(
            r#"effects = flag !->
    if flag
        match flag
            true => ()
            false => 7 as I32
    else
        9 as I64

main = !->
    effects true
"#,
        )
        .expect("nested control flow inherits only the outer effect context");
    }

    #[test]
    fn discarded_context_does_not_leak_into_bindings_or_value_returning_lambdas() {
        for branch in [
            "if flag\n        ()\n    else\n        7 as I32",
            "match flag\n        true => ()\n        false => 7 as I32",
        ] {
            let source = format!(
                "effects = flag !->\n    value = {branch}\n\nmain = !->\n    effects true\n"
            );
            let errors =
                resolved_source(&source).expect_err("binding initializer remains a value context");
            assert!(errors
                .iter()
                .any(|error| error.message.contains("Type mismatch")));
        }
        let errors = resolved_source(
            r#"main = !->
    worker = flag ->
        if flag
            ()
        else
            7 as I32
"#,
        )
        .expect_err("ordinary lambda return remains a value context");
        assert!(errors
            .iter()
            .any(|error| error.message.contains("Type mismatch")));
        let errors = resolved_source(
            "effects: Bool -> ()\neffects = flag ->\n    if flag\n        ()\n    else\n        7 as I32\n\nmain = !->\n    effects true\n",
        ).expect_err("a declared unit return is not an explicit unit-arrow discard contract");
        assert!(errors
            .iter()
            .any(|error| error.message.contains("Type mismatch")));
        let errors = resolved_source(
            "choose: Bool -> I32\nchoose = flag ->\n    return if flag\n        ()\n    else\n        7 as I32\n\nmain = !->\n    choose true\n",
        ).expect_err("an explicit return operand remains a value context");
        assert!(errors
            .iter()
            .any(|error| error.message.contains("Type mismatch")));
    }

    #[test]
    fn discarded_owned_branch_results_reach_typed_mir_cleanup() {
        for branch in [
            "    if flag\n        ()\n    else\n        make_owned!",
            "    match flag\n        true => ()\n        false => make_owned!",
        ] {
            let source = format!(
                r#"lang drop
< trait Release
    lang method
    ~@release: ()

struct Owned
    < value: I64

impl Release for Owned
    ~@release = !-> ()

make_owned: Owned
make_owned = -> Owned
    value: 7

effects = flag !->
{branch}

main = !->
    effects false
"#
            );
            let resolved = resolved_source(&source).expect("owned effect branch finalizes");
            let (owned_id, _) = resolved.program.test_struct_by_name("Owned").unwrap();
            let (effects_id, effects) = resolved.program.test_function_by_name("effects").unwrap();
            let owned_ty = Type::Struct {
                id: owned_id,
                args: Vec::new(),
            };
            assert!(!TypeFacts::is_copy(&owned_ty));
            let crate::hir::HirStmtFor::Expr(join) = &effects.body.stmts[0] else {
                panic!("expected discarded branch");
            };
            let owned_branch = match &join.kind {
                crate::hir::HirExprKindFor::If { else_branch, .. } => else_branch.as_ref().unwrap(),
                crate::hir::HirExprKindFor::Match { arms, .. } => &arms[1].body,
                _ => panic!("expected if or match"),
            };
            let crate::hir::HirStmtFor::Expr(original) = &owned_branch.stmts[0] else {
                panic!("expected owned-producing call");
            };
            assert_eq!(original.ty, owned_ty);
            assert!(matches!(
                original.kind,
                crate::hir::HirExprKindFor::Call(..)
            ));
            assert_eq!(owned_branch.ty, Type::Unit);

            let mono = crate::mono::monomorphize(resolved.program, resolved.source_map)
                .expect("owned discard monomorphizes through the canonical pipeline");
            let effects_instance = mono
                .instances
                .iter()
                .find_map(|(id, record)| {
                    (record.origin == crate::mono::InstanceOrigin::Function(effects_id))
                        .then_some(*id)
                })
                .expect("effects has a canonical instance");
            let mir = crate::mir::builder::MirBuilder::build_monomorphized(&mono);
            let agreement = crate::mir::agreement::check_mir_runtime_agreement(&mir);
            assert!(agreement.is_clean(), "{agreement:?}");
            crate::mir::borrowck::BorrowChecker::run(&mir)
                .expect("discarded owned value satisfies native ownership validation");
            let function = mir
                .function(crate::mir::MirFunctionId::Instance(effects_instance))
                .unwrap();
            let drops = function
                .basic_blocks
                .iter()
                .filter_map(|block| match &block.terminator {
                    Some(
                        crate::mir::Terminator::Drop { place, .. }
                        | crate::mir::Terminator::DropWithOrigin { place, .. },
                    ) if mir
                        .type_context
                        .type_for(function.local_decls[place.local.0].ty)
                        == owned_ty =>
                    {
                        Some(place.local)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                drops.len(),
                1,
                "the one owned result must be dropped exactly once, not retagged as unit"
            );
            for local in drops {
                assert_eq!(
                    function.local_decls[local.0].source,
                    crate::mir::LocalSource::Temporary
                );
                assert!(function.basic_blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                    matches!(statement.kind, crate::mir::StatementKind::StorageDead(dead) if dead == local)
                }));
            }
        }
    }
}

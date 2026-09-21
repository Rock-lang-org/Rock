//! Expression lowering methods for Lowerer

use std::collections::{HashMap, HashSet};

use crate::ast;
use crate::hir::*;
use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::selection::{ReceiverCandidate, SelectedMethod, SelectionDiagnostic};
use crate::types::{GenericParamId, Type};

use crate::lower::Lowerer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprUse {
    Value,
    AssignmentPlace,
}

pub(crate) enum AstExprOperand<'a> {
    Unary(&'a ast::UnaryExpr),
    Cast(&'a ast::Expression),
}

fn ast_expr_operand_span(operand: &AstExprOperand<'_>) -> Option<Span> {
    fn path_span(path: &[ast::IdentOrType]) -> Option<Span> {
        path.first().map(|segment| match segment {
            ast::IdentOrType::Ident(ident) => ident.span.clone(),
            ast::IdentOrType::Type(ty) => ty.span(),
        })
    }

    fn operand_span(operand: &ast::Operand) -> Option<Span> {
        match operand {
            ast::Operand::Literal(literal) => Some(literal.span.clone()),
            ast::Operand::Ident(path) => path_span(&path.path),
            ast::Operand::CallHole(span) => Some(span.clone()),
            ast::Operand::SelfIdent(ident) => Some(ident.span.clone()),
            ast::Operand::Instance(instance) => path_span(&instance.name.path),
            ast::Operand::Tuple(tuple) => tuple.elements.first().and_then(ast_expr_span),
            ast::Operand::Expression(expr) => ast_expr_span(expr),
            ast::Operand::If(if_expr) => ast_expr_span(&if_expr.condition.expression),
            ast::Operand::Match(match_expr) => ast_expr_span(&match_expr.expr),
            ast::Operand::Loop(_) | ast::Operand::LambdaDecl(_) | ast::Operand::Unsafe(_, _) => {
                None
            }
            ast::Operand::NativeOperator(operator) => Some(operator.span.clone()),
        }
    }

    fn ast_unary_span(unary: &ast::UnaryExpr) -> Option<Span> {
        match unary {
            ast::UnaryExpr::PrimaryExpr(primary) => operand_span(&primary.operand),
            ast::UnaryExpr::UnaryExpr(operator, _) => Some(operator.span.clone()),
        }
    }

    fn ast_expr_span(expression: &ast::Expression) -> Option<Span> {
        match expression {
            ast::Expression::BinopExpr(_, operator, _) => Some(operator.span.clone()),
            ast::Expression::UnaryExpr(unary) => ast_unary_span(unary),
            ast::Expression::CastExpr(_, ty) => Some(ty.span()),
            ast::Expression::Range(range) => Some(range.span.clone()),
        }
    }

    match operand {
        AstExprOperand::Unary(unary) => ast_unary_span(unary),
        AstExprOperand::Cast(expression) => ast_expr_span(expression),
    }
}

const APPLICATION_PRECEDENCE: u8 = 8;

impl Lowerer {
    // ============================================================
    // Expression Lowering Methods
    // ============================================================

    pub(crate) fn op_precedence(&self, op: &str) -> Result<u8, String> {
        self.infix_precedence.get(op).copied().ok_or_else(|| {
            format!(
                "Operator '{}' has no precedence declaration (use 'infix N {}')",
                op, op
            )
        })
    }

    pub(crate) fn instantiate_function_type(&mut self, func: &HirFunction, span: Span) -> Type {
        if func.generic_params.is_empty()
            && self.should_instantiate_inferred_function(func.id)
            && self
                .function_type_vars
                .get(&func.id)
                .is_some_and(|vars| !vars.is_empty())
        {
            let mut representatives = HashMap::<TypeVarId, Type>::new();
            let mut substitution = HashMap::<TypeVarId, Type>::new();
            for variable in self
                .function_type_vars
                .get(&func.id)
                .into_iter()
                .flatten()
                .copied()
            {
                let resolved = self.engine.resolve(&Type::TypeVar(variable));
                let replacement = match resolved {
                    Type::TypeVar(representative) => representatives
                        .entry(representative)
                        .or_insert_with(|| {
                            let kind = self.engine.kind_of_type_var(representative);
                            self.engine.fresh_type_var_at_kind(span.clone(), kind)
                        })
                        .clone(),
                    resolved => resolved,
                };
                substitution.insert(variable, replacement);
            }

            let params = func
                .params
                .iter()
                .map(|param| param.ty.substitute(&substitution))
                .collect();
            let ret = func.ret_type.substitute(&substitution);
            return Type::function_with_safety(
                params,
                ret,
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
        }

        let mut generic_params = HashSet::new();
        // Parameters used only in bounds still need fresh call-site variables
        // (for example T in `C: ForEach T, A: FnMut T, ()`).
        generic_params.extend(func.generic_params.iter().map(|param| param.id));
        for param in &func.params {
            param.ty.collect_generic_params(&mut generic_params);
        }
        func.ret_type.collect_generic_params(&mut generic_params);

        let subst: HashMap<GenericParamId, Type> = generic_params
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

        let context = format!("call to generic function '{}'", func.name);
        for (generic_param, bounds) in &func.generic_bounds {
            let Some(ty) = subst.get(generic_param).cloned() else {
                continue;
            };
            for bound in bounds {
                let instantiated_bound = crate::types::TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(&subst))
                        .collect(),
                };
                if let Type::TypeVar(id) = &ty {
                    self.engine.add_bound(*id, instantiated_bound.clone());
                }
                self.constraint_store.add_trait(
                    ty.clone(),
                    instantiated_bound,
                    span.clone(),
                    &context,
                );
            }
        }

        let params = func
            .params
            .iter()
            .map(|param| param.ty.substitute_generics(&subst))
            .collect();
        let ret = func.ret_type.substitute_generics(&subst);
        Type::function_with_safety(
            params,
            ret,
            crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
        )
    }

    fn infer_bounded_constructor_heads(&mut self, function_ty: &Type, args: &[&HirExpr]) {
        let Type::Function { params, .. } = function_ty else {
            return;
        };

        for (expected, actual) in params.iter().zip(args) {
            let resolved_expected = self.engine.resolve(expected);
            let Type::Apply {
                ref constructor, ..
            } = resolved_expected
            else {
                continue;
            };
            let Type::TypeVar(constructor_id) = self.engine.resolve(constructor) else {
                continue;
            };
            let bounds = self.engine.get_bounds(constructor_id);
            if bounds.is_empty() {
                continue;
            }

            let actual_ty = self.resolve_projection_type(&self.engine.resolve(&actual.ty));
            if let Type::TypeVar(actual_id) = actual_ty {
                if let Err(error) = self
                    .engine
                    .bind_pending_type_var(actual_id, &resolved_expected)
                {
                    self.diagnostics
                        .push_type_with_span(error.render(&self.engine), actual.span.clone());
                }
                continue;
            }
            let mut targets = Vec::new();
            for bound in bounds {
                let trait_args = bound
                    .type_args
                    .iter()
                    .map(|arg| self.engine.resolve(arg))
                    .collect::<Vec<_>>();
                let result = self
                    .selection_service()
                    .infer_constructor_target_for_applied_type(
                        &actual_ty,
                        bound.trait_id,
                        &trait_args,
                    );
                match result {
                    Ok(Some(target)) => targets.push(target),
                    Ok(None) => {}
                    Err(error) => {
                        let message = self.display_selection_error(&error);
                        self.diagnostics
                            .push_selection_with_span(message, actual.span.clone());
                        return;
                    }
                }
            }
            targets.sort_by_key(ToString::to_string);
            targets.dedup();
            if let [target] = targets.as_slice() {
                if let Err(error) = self.engine.unify(&Type::TypeVar(constructor_id), target) {
                    self.diagnostics
                        .push_type_with_span(error.render(&self.engine), actual.span.clone());
                }
            }
        }
    }

    pub(crate) fn instantiate_resolved_value_type(
        &mut self,
        resolved: &crate::lower::resolution::LowerResolvedValue,
        span: Span,
    ) -> Type {
        if let Some(HirVarTarget::Function(function_id)) = resolved.target {
            if let Some(func) = self.items.function(function_id).cloned() {
                return self.instantiate_function_type(&func, span);
            }
        }

        if resolved.should_instantiate {
            self.instantiate_generics(resolved.ty.clone(), span)
        } else {
            resolved.ty.clone()
        }
    }

    fn autoref_operator_arg_for_selected_method(
        &mut self,
        arg: HirExpr,
        selected: &crate::selection::SelectedMethod,
        index: usize,
    ) -> HirExpr {
        let Some(param) = selected.substituted_params.get(index) else {
            return arg;
        };
        let expected_ty = self.resolve_projection_type(&param.ty);
        let resolved_arg_ty = self.resolve_projection_type(&self.engine.resolve(&arg.ty));

        if matches!(expected_ty, Type::Reference { mutable: false, .. })
            && expected_ty != resolved_arg_ty
        {
            let span = arg.span.clone();
            return HirExpr {
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(resolved_arg_ty),
                },
                kind: HirExprKind::Ref(false, Box::new(arg)),
                span,
            };
        }

        arg
    }

    fn operator_method_accepts_args(
        &self,
        selected: &crate::selection::SelectedMethod,
        args: &[HirExpr],
    ) -> bool {
        if selected.substituted_params.len() != args.len() {
            return false;
        }

        if selected
            .substituted_params
            .iter()
            .any(|param| Self::type_has_unresolved_parameter(&param.ty))
        {
            return true;
        }

        let mut subst = HashMap::new();
        selected
            .substituted_params
            .iter()
            .zip(args)
            .all(|(param, arg)| {
                let expected = self.resolve_projection_type(&param.ty);
                let actual = self.resolve_projection_type(&self.engine.resolve(&arg.ty));
                if crate::selection::type_pattern_matches(&expected, &actual, &mut subst) {
                    return true;
                }
                if Self::type_accepts_inference_vars(&expected, &actual) {
                    return true;
                }
                if let Type::Reference {
                    inner,
                    mutable: false,
                } = expected
                {
                    return crate::selection::type_pattern_matches(&inner, &actual, &mut subst)
                        || Self::type_accepts_inference_vars(&inner, &actual);
                }
                false
            })
    }

    fn report_unsafe_operator_method_call_if_needed(
        &mut self,
        method_func: &HirFunction,
        method_name: &str,
        span: Span,
    ) {
        if method_func.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Call to unsafe function '{}' requires an unsafe block",
                    method_name
                ),
                span,
            );
        }
    }

    fn report_unsafe_operator_function_call_if_needed(
        &mut self,
        target: Option<&HirVarTarget>,
        function_name: &str,
        span: Span,
    ) {
        let is_unsafe = match target {
            Some(HirVarTarget::Function(function_id)) => self
                .items
                .function(*function_id)
                .is_some_and(|func| func.is_unsafe),
            _ => self
                .resolve_module_alias_or_item_def_id(function_name)
                .and_then(|id| self.items.function(id))
                .is_some_and(|func| func.is_unsafe),
        };

        if is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Call to unsafe function '{}' requires an unsafe block",
                    function_name
                ),
                span,
            );
        }
    }

    fn type_accepts_inference_vars(expected: &Type, actual: &Type) -> bool {
        match (expected, actual) {
            (Type::TypeVar(_), _) | (_, Type::TypeVar(_)) => true,
            (
                Type::Function {
                    params: expected_args,
                    ret: expected_ret,
                    ..
                },
                Type::Function {
                    params: actual_args,
                    ret: actual_ret,
                    ..
                },
            ) => {
                expected_args.len() == actual_args.len()
                    && expected_args
                        .iter()
                        .zip(actual_args)
                        .all(|(expected, actual)| {
                            Self::type_accepts_inference_vars(expected, actual)
                        })
                    && Self::type_accepts_inference_vars(expected_ret, actual_ret)
            }
            (
                Type::Struct {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Struct {
                    id: actual_id,
                    args: actual_args,
                },
            )
            | (
                Type::Enum {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Enum {
                    id: actual_id,
                    args: actual_args,
                },
            ) => {
                expected_id == actual_id
                    && expected_args.len() == actual_args.len()
                    && expected_args
                        .iter()
                        .zip(actual_args)
                        .all(|(expected, actual)| {
                            Self::type_accepts_inference_vars(expected, actual)
                        })
            }
            (
                Type::Reference {
                    mutable: expected_mutable,
                    inner: expected_inner,
                },
                Type::Reference {
                    mutable: actual_mutable,
                    inner: actual_inner,
                },
            ) => {
                (!expected_mutable || *actual_mutable)
                    && Self::type_accepts_inference_vars(expected_inner, actual_inner)
            }
            (Type::Slice(expected), Type::Slice(actual))
            | (Type::Pointer(expected), Type::Pointer(actual)) => {
                Self::type_accepts_inference_vars(expected, actual)
            }
            (Type::Array(expected, expected_len), Type::Array(actual, actual_len)) => {
                expected_len == actual_len && Self::type_accepts_inference_vars(expected, actual)
            }
            _ => expected == actual,
        }
    }

    fn type_has_unresolved_parameter(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(nested, Type::TypeVar(_) | Type::Generic(_))
        })
    }

    pub(crate) fn lower_expression(&mut self, expr: &ast::Expression) -> HirExpr {
        let lowered = self.lower_expression_with_use(expr, ExprUse::Value);
        self.source_map.record_expression(lowered.span.clone());
        lowered
    }

    fn flatten_owned_binop(
        expr: &ast::Expression,
        operands: &mut Vec<ast::Expression>,
        operators: &mut Vec<ast::Operator>,
    ) {
        match expr {
            ast::Expression::BinopExpr(lhs, operator, rhs) => {
                operands.push(ast::Expression::UnaryExpr(lhs.clone()));
                operators.push(operator.clone());
                Self::flatten_owned_binop(rhs, operands, operators);
            }
            _ => operands.push(expr.clone()),
        }
    }

    fn build_owned_binop_chain(
        operands: &[ast::Expression],
        operators: &[ast::Operator],
    ) -> Option<ast::Expression> {
        let mut expression = operands.last()?.clone();
        for index in (0..operators.len()).rev() {
            let ast::Expression::UnaryExpr(lhs) = operands[index].clone() else {
                return None;
            };
            expression =
                ast::Expression::BinopExpr(lhs, operators[index].clone(), Box::new(expression));
        }
        Some(expression)
    }

    fn append_ast_secondaries(
        expression: ast::Expression,
        mut trailing: Vec<ast::SecondaryExpr>,
    ) -> ast::Expression {
        if trailing.is_empty() {
            return expression;
        }

        if let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(mut primary)) = expression {
            primary
                .secondaries
                .get_or_insert_with(Vec::new)
                .append(&mut trailing);
            ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary))
        } else {
            ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(ast::PrimaryExpr {
                operand: ast::Operand::Expression(Box::new(expression)),
                secondaries: Some(trailing),
                type_annotation: None,
            }))
        }
    }

    fn apply_application_precedence(&self, expr: &ast::Expression) -> Option<ast::Expression> {
        match expr {
            ast::Expression::BinopExpr(lhs, operator, rhs) => {
                if let Some(rhs) = self.apply_application_precedence(rhs) {
                    return Some(ast::Expression::BinopExpr(
                        lhs.clone(),
                        operator.clone(),
                        Box::new(rhs),
                    ));
                }

                let lhs_expression = ast::Expression::UnaryExpr(lhs.clone());
                if let Some(lhs) = self.apply_application_precedence(&lhs_expression) {
                    let mut operands = Vec::new();
                    let mut operators = Vec::new();
                    Self::flatten_owned_binop(&lhs, &mut operands, &mut operators);
                    operators.push(operator.clone());
                    operands.push((**rhs).clone());
                    return Self::build_owned_binop_chain(&operands, &operators);
                }
            }
            ast::Expression::CastExpr(inner, ty) => {
                if let Some(inner) = self.apply_application_precedence(inner) {
                    return Some(ast::Expression::CastExpr(Box::new(inner), ty.clone()));
                }
            }
            ast::Expression::UnaryExpr(_) => {}
            ast::Expression::Range(_) => {}
        }

        let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary)) = expr else {
            return None;
        };
        let secondaries = primary.secondaries.as_ref()?;

        for (secondary_index, secondary) in secondaries.iter().enumerate() {
            let ast::SecondaryExpr::Arguments(arguments) = secondary else {
                continue;
            };

            for (argument_index, argument) in arguments.iter().enumerate() {
                let Some(argument) = self.apply_application_precedence(&argument.arg) else {
                    continue;
                };

                let mut primary = primary.clone();
                let secondaries = primary.secondaries.as_mut()?;
                let ast::SecondaryExpr::Arguments(arguments) = &mut secondaries[secondary_index]
                else {
                    unreachable!();
                };
                arguments[argument_index].arg = argument;
                return Some(ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(
                    primary,
                )));
            }
        }

        for (secondary_index, secondary) in secondaries.iter().enumerate() {
            let ast::SecondaryExpr::Arguments(arguments) = secondary else {
                continue;
            };
            let Some(last_argument) = arguments.last() else {
                continue;
            };

            let mut operands = Vec::new();
            let mut operators = Vec::new();
            Self::flatten_owned_binop(&last_argument.arg, &mut operands, &mut operators);
            let Some(split_at) = operators.iter().position(|operator| {
                self.infix_precedence
                    .get(&operator.value)
                    .is_some_and(|precedence| *precedence <= APPLICATION_PRECEDENCE)
            }) else {
                continue;
            };

            let argument =
                Self::build_owned_binop_chain(&operands[..=split_at], &operators[..split_at])?;
            let mut call_secondaries = secondaries[..=secondary_index].to_vec();
            let ast::SecondaryExpr::Arguments(call_arguments) =
                &mut call_secondaries[secondary_index]
            else {
                unreachable!();
            };
            call_arguments.last_mut()?.arg = argument;

            let mut call_primary = primary.clone();
            call_primary.secondaries = Some(call_secondaries);
            let call = ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(call_primary));

            let mut outer_operands = vec![call];
            outer_operands.extend_from_slice(&operands[split_at + 1..]);
            let outer = Self::build_owned_binop_chain(&outer_operands, &operators[split_at..])?;
            let trailing = secondaries[secondary_index + 1..].to_vec();
            return Some(Self::append_ast_secondaries(outer, trailing));
        }

        None
    }

    fn lower_expression_with_use(&mut self, expr: &ast::Expression, use_kind: ExprUse) -> HirExpr {
        if let ast::Expression::Range(range) = expr {
            if use_kind == ExprUse::AssignmentPlace {
                self.diagnostics.push_type_with_span(
                    "Range expressions cannot be used as assignment places".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            }

            let Some(range_items) = self.language_items.range.clone() else {
                self.diagnostics.push_with_span(
                    "Cannot use range syntax because the Range language-item protocol is unavailable"
                        .to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };

            let start = range
                .start
                .as_ref()
                .map(|start| self.lower_expression(start));
            let end = range.end.as_ref().map(|end| self.lower_expression(end));
            if let Some(start) = &start {
                let _ = self.engine.unify(&start.ty, &Type::I64);
            }
            if let Some(end) = &end {
                let _ = self.engine.unify(&end.ty, &Type::I64);
            }

            let variant_id = match (start.is_some(), end.is_some(), range.inclusive) {
                (false, false, false) => range_items.full_variant_id,
                (true, false, false) => range_items.from_variant_id,
                (false, true, false) => range_items.to_variant_id,
                (false, true, true) => range_items.to_inclusive_variant_id,
                (true, true, false) => range_items.exclusive_variant_id,
                (true, true, true) => range_items.inclusive_variant_id,
                (_, false, true) => unreachable!("parser rejects inclusive ranges without an end"),
            };
            let Some(enumeration) = self.items.enumeration(range_items.enum_id) else {
                self.diagnostics.push_with_span(
                    "Range language item refers to an unavailable enum".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };
            let Some(variant) = enumeration
                .variants
                .iter()
                .find(|variant| variant.id == variant_id)
            else {
                self.diagnostics.push_with_span(
                    "Range language item refers to an unavailable variant".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };
            let enum_name = enumeration.name.clone();
            let variant_name = variant.name.clone();
            let mut args = Vec::new();
            if let Some(start) = start {
                args.push(start);
            }
            if let Some(end) = end {
                args.push(end);
            }
            return HirExpr {
                ty: Type::Enum {
                    id: range_items.enum_id,
                    args: Vec::new(),
                },
                kind: HirExprKind::EnumVariant(
                    enum_name,
                    variant_name.clone(),
                    args,
                    Some(HirVariantLocation {
                        owner: range_items.enum_id,
                        variant_id,
                        name: variant_name,
                    }),
                ),
                span: range.span.clone(),
            };
        }

        if let Some(expression) = self.apply_application_precedence(expr) {
            return self.lower_expression_with_use(&expression, use_kind);
        }

        // Handle `expr as Type` casts
        if let ast::Expression::CastExpr(inner, parse_ty) = expr {
            if use_kind == ExprUse::AssignmentPlace {
                self.diagnostics.push_type_with_span(
                    "Cast expressions cannot be used as assignment places".to_string(),
                    parse_ty.span(),
                );
                return self.error_expression_at(parse_ty.span());
            }
            let inner_hir = self.lower_expression_with_use(inner, use_kind);
            let target_ty = self.lower_parse_type(parse_ty);
            let resolved_inner_ty =
                self.resolve_projection_type(&self.engine.resolve(&inner_hir.ty));
            let resolved_target_ty = self.resolve_projection_type(&self.engine.resolve(&target_ty));

            if Self::is_fat_raw_slice_pointer(&resolved_inner_ty) && target_ty.is_integer() {
                self.diagnostics.push_type_with_span(
                    "Cannot cast fat raw slice pointer to integer".to_string(),
                    parse_ty.span(),
                );
            }

            if resolved_inner_ty.is_integer() && Self::is_fat_raw_slice_pointer(&resolved_target_ty)
            {
                self.diagnostics.push_with_span(
                    "Cannot cast integer to fat raw slice pointer".to_string(),
                    parse_ty.span(),
                );
            }

            return HirExpr {
                ty: target_ty.clone(),
                kind: HirExprKind::Cast(Box::new(inner_hir), target_ty),
                span: parse_ty.span(),
            };
        }

        // Flatten the right-recursive BinopExpr chain into a list
        let mut operands: Vec<AstExprOperand<'_>> = Vec::new();
        let mut operators: Vec<String> = Vec::new();

        self.flatten_binop(expr, &mut operands, &mut operators);

        if operands.len() == 1 {
            return self.lower_ast_expr_operand_with_use(&operands[0], use_kind);
        }

        // Build tree respecting precedence using precedence climbing
        self.build_precedence_tree_with_use(&operands, &operators, 0, operands.len() - 1, use_kind)
    }

    /// Flatten a right-recursive BinopExpr chain into operands and operators
    pub(crate) fn flatten_binop<'a>(
        &mut self,
        expr: &'a ast::Expression,
        operands: &mut Vec<AstExprOperand<'a>>,
        operators: &mut Vec<String>,
    ) {
        match expr {
            ast::Expression::BinopExpr(lhs, op, rhs) => {
                operands.push(AstExprOperand::Unary(lhs));
                operators.push(op.value.clone());
                self.flatten_binop(rhs, operands, operators);
            }
            ast::Expression::UnaryExpr(unary) => {
                operands.push(AstExprOperand::Unary(unary));
            }
            ast::Expression::CastExpr(_, _) => {
                operands.push(AstExprOperand::Cast(expr));
            }
            ast::Expression::Range(_) => {
                unreachable!("ranges are lowered before binary flattening")
            }
        }
    }

    fn lower_ast_expr_operand_with_use(
        &mut self,
        operand: &AstExprOperand<'_>,
        use_kind: ExprUse,
    ) -> HirExpr {
        match operand {
            AstExprOperand::Unary(unary) => {
                if use_kind == ExprUse::Value {
                    self.lower_unary_expr(unary)
                } else {
                    self.lower_unary_expr_with_use(unary, use_kind)
                }
            }
            AstExprOperand::Cast(expr) => self.lower_expression_with_use(expr, use_kind),
        }
    }

    fn build_precedence_tree_with_use(
        &mut self,
        operands: &[AstExprOperand<'_>],
        operators: &[String],
        start: usize,
        end: usize,
        use_kind: ExprUse,
    ) -> HirExpr {
        if start == end {
            return self.lower_ast_expr_operand_with_use(&operands[start], use_kind);
        }

        let mut min_prec = u8::MAX;
        let mut split_at = start;
        for i in start..end {
            let prec = match self.op_precedence(&operators[i]) {
                Ok(prec) => prec,
                Err(error) => {
                    let Some(span) = ast_expr_operand_span(&operands[start]) else {
                        self.diagnostics.push_toolchain(error);
                        return self.lower_ast_expr_operand_with_use(&operands[start], use_kind);
                    };
                    self.diagnostics.push_with_span(error, span.clone());
                    return self.error_expression_at(span);
                }
            };
            if prec <= min_prec {
                min_prec = prec;
                split_at = i;
            }
        }

        let op = &operators[split_at];
        let (left_use, right_use) = if op == "=" {
            (ExprUse::AssignmentPlace, ExprUse::Value)
        } else {
            (ExprUse::Value, ExprUse::Value)
        };
        let left =
            self.build_precedence_tree_with_use(operands, operators, start, split_at, left_use);
        let right =
            self.build_precedence_tree_with_use(operands, operators, split_at + 1, end, right_use);

        self.build_precedence_tree(&[left, right], std::slice::from_ref(op), 0, 1)
    }

    /// Build a correctly-precedenced tree from a flat list using
    /// a simple algorithm: find the lowest-precedence operator and split there
    pub(crate) fn build_precedence_tree(
        &mut self,
        operands: &[HirExpr],
        operators: &[String],
        start: usize,
        end: usize,
    ) -> HirExpr {
        if start == end {
            return operands[start].clone();
        }

        // Find the lowest precedence operator in range [start..end-1]
        // For left-associativity, take the RIGHTMOST occurrence of the lowest precedence
        let op_start = start;
        let op_end = end; // operators[i] is between operands[i] and operands[i+1]

        let mut min_prec = u8::MAX;
        let mut split_at = op_start;

        for i in op_start..op_end {
            let prec = match self.op_precedence(&operators[i]) {
                Ok(p) => p,
                Err(e) => {
                    let span = operands[start].span.clone();
                    self.diagnostics.push_with_span(e, span.clone());
                    return self.error_expression_at(span);
                }
            };
            if prec <= min_prec {
                min_prec = prec;
                split_at = i;
            }
        }

        let left = self.build_precedence_tree(operands, operators, start, split_at);
        let right = self.build_precedence_tree(operands, operators, split_at + 1, end);

        let op_str = &operators[split_at];

        // Assignment operator: special handling
        if op_str == "=" {
            if let Err(error) = self.engine.unify(&right.ty, &left.ty) {
                self.diagnostics.push_type_with_span(
                    format!("Assignment type mismatch: {}", error.render(&self.engine)),
                    right.span.clone(),
                );
            }
            let operation_span = right.span.clone();
            return HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Assign(Box::new(left), Box::new(right)),
                span: operation_span,
            };
        }

        // Short-circuit operators: keep as BinOp for special evaluation semantics
        if op_str == "&&" || op_str == "||" {
            // Comparison/logical ops: operands should be compatible
            if let Err(e) = self.engine.unify(&left.ty, &right.ty) {
                self.diagnostics.push_type_with_span(
                    format!(
                        "Binary operator '{}': operand type mismatch: {}",
                        op_str,
                        e.render(&self.engine)
                    ),
                    right.span.clone(),
                );
            }
            let operation_span = right.span.clone();
            let bin_op = if op_str == "&&" {
                BinOp::And
            } else {
                BinOp::Or
            };
            return HirExpr {
                ty: Type::Bool,
                kind: HirExprKind::BinOp(bin_op, Box::new(left), Box::new(right)),
                span: operation_span,
            };
        }

        let resolved_left_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));

        if op_str == "&" {
            let resolved_fn_ty = self.resolve_projection_type(&resolved_left_ty);
            if let Type::Function {
                params,
                ret,
                safety,
                ..
            } = &resolved_fn_ty
            {
                let right_span = right.span.clone();
                let borrowed_right = HirExpr {
                    ty: Type::Reference {
                        mutable: false,
                        inner: Box::new(right.ty.clone()),
                    },
                    kind: HirExprKind::Ref(false, Box::new(right)),
                    span: right_span.clone(),
                };
                let arg = params
                    .first()
                    .map(|param| self.coerce_argument_to_expected(borrowed_right.clone(), param))
                    .unwrap_or(borrowed_right);
                let ret_ty = if params.len() == 1 {
                    ret.as_ref().clone()
                } else {
                    self.engine.fresh_type_var_at(right_span)
                };
                let expected_fn_ty =
                    Type::function_with_safety(vec![arg.ty.clone()], ret_ty.clone(), *safety);
                let _ = self.engine.unify(&left.ty, &expected_fn_ty);
                self.report_unsafe_function_value_call_from_type(&left);

                let left_is_unsafe_function = matches!(
                    self.resolve_projection_type(&self.engine.resolve(&left.ty)),
                    Type::Function {
                        safety: crate::types::FunctionSafety::Unsafe,
                        ..
                    }
                );
                if !left_is_unsafe_function {
                    match &left.kind {
                        HirExprKind::ResolvedVar(reference) => self
                            .report_unsafe_operator_function_call_if_needed(
                                Some(&reference.target),
                                &reference.name,
                                left.span.clone(),
                            ),
                        HirExprKind::Var(function_name) => self
                            .report_unsafe_operator_function_call_if_needed(
                                None,
                                function_name,
                                left.span.clone(),
                            ),
                        _ => {}
                    }
                }

                let target = self.call_target_for_callee(&left);
                let operation_span = left.span.clone();
                return HirExpr {
                    ty: self.resolve_projection_type(&self.engine.resolve(&ret_ty)),
                    kind: HirExprKind::Call(Box::new(left), vec![arg], target),
                    span: operation_span,
                };
            }
        }

        let mut resolved_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));

        if let Type::Generic(gen_param) = &resolved_ty {
            let bounds = self
                .current_impl_bounds()
                .get(gen_param)
                .cloned()
                .unwrap_or_default();
            let receiver_candidates = self.receiver_adjustment_candidates(left.clone());

            let result = self
                .selection_service()
                .select_bound_method_preferring_non_ref_receiver(
                    &receiver_candidates,
                    &bounds,
                    op_str,
                    resolved_ty.clone(),
                    matches!(&left.kind, HirExprKind::Call(_, _, _)),
                );
            let span = left.span.clone();
            if let Some(selected) = self.handle_optional_selection(result, span) {
                let adjusted_recv = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );
                let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
                let Some((method_func, ret_ty, coerced_args, target)) =
                    self.selected_method_call_types(&selected, vec![right])
                else {
                    return self.error_expression_at(left.span.clone());
                };
                self.report_unsafe_operator_method_call_if_needed(
                    &method_func,
                    op_str,
                    left.span.clone(),
                );

                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::MethodCall(
                        Box::new(adjusted_recv),
                        op_str.clone(),
                        coerced_args,
                        method_func.self_receiver,
                        Some(target),
                    ),
                    span: left.span.clone(),
                };
            }
        }

        if let Type::TypeVar(id) = &resolved_ty {
            let bounds = self.engine.get_bounds(*id);
            let receiver_candidates = self.receiver_adjustment_candidates(left.clone());
            let result = self
                .selection_service()
                .select_bound_method_preferring_non_ref_receiver(
                    &receiver_candidates,
                    &bounds,
                    op_str,
                    resolved_ty.clone(),
                    matches!(&left.kind, HirExprKind::Call(_, _, _)),
                );
            let span = left.span.clone();
            if let Some(selected) = self.handle_optional_selection(result, span) {
                let adjusted_recv = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );
                let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
                let Some((method_func, ret_ty, coerced_args, target)) =
                    self.selected_method_call_types(&selected, vec![right])
                else {
                    return self.error_expression_at(left.span.clone());
                };
                self.report_unsafe_operator_method_call_if_needed(
                    &method_func,
                    op_str,
                    left.span.clone(),
                );

                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::MethodCall(
                        Box::new(adjusted_recv),
                        op_str.clone(),
                        coerced_args,
                        method_func.self_receiver,
                        Some(target),
                    ),
                    span: left.span.clone(),
                };
            }
        }

        let func_binding = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_identifier_value(op_str.as_str())
            .map(|resolved| {
                let func_ty = self.instantiate_resolved_value_type(&resolved, left.span.clone());
                (func_ty, resolved.name, resolved.target)
            });

        if let Some((func_ty, hir_name, target)) = func_binding {
            self.infer_bounded_constructor_heads(&func_ty, &[&left, &right]);
            let resolved_func_ty = self.resolve_projection_type(&self.engine.resolve(&func_ty));
            let (mut expected_ret_ty, safety) = match resolved_func_ty {
                Type::Function { ret, safety, .. } => (*ret, safety),
                _ => (
                    self.engine.fresh_type_var_at(left.span.clone()),
                    crate::types::FunctionSafety::Safe,
                ),
            };
            let expected_fn_ty = Type::function_with_safety(
                vec![left.ty.clone(), right.ty.clone()],
                expected_ret_ty.clone(),
                safety,
            );
            if let Err(e) = self.engine.unify(&func_ty, &expected_fn_ty) {
                if matches!(
                    &e,
                    crate::infer::UnifyError::ConstructorInference { .. }
                        | crate::infer::UnifyError::AmbiguousConstructorHeads { .. }
                ) && (Self::type_has_unresolved_parameter(&left.ty)
                    || Self::type_has_unresolved_parameter(&right.ty))
                {
                    let span = left.span.clone();
                    self.constraint_store.add_equality(
                        func_ty.clone(),
                        expected_fn_ty.clone(),
                        span,
                        &format!("Operator '{op_str}' function type mismatch"),
                    );
                } else {
                    self.diagnostics.push_type_with_span(
                        format!(
                            "Operator '{}': function type mismatch: {}",
                            op_str,
                            e.render(&self.engine)
                        ),
                        left.span.clone(),
                    );
                }
            }

            expected_ret_ty = match self.engine.normalize_resolved_type(&expected_ret_ty) {
                Ok(ty) => ty,
                Err(error) => {
                    self.diagnostics
                        .push_type_with_span(error, left.span.clone());
                    Type::Error
                }
            };
            self.report_unsafe_operator_function_call_if_needed(
                target.as_ref(),
                &hir_name,
                left.span.clone(),
            );

            let operation_span = left.span.clone();
            return HirExpr {
                ty: expected_ret_ty.clone(),
                kind: {
                    let callee = HirExpr {
                        ty: self.engine.resolve(&func_ty),
                        kind: match target {
                            Some(target) => HirExprKind::ResolvedVar(HirVarRef {
                                name: hir_name,
                                target,
                            }),
                            None => HirExprKind::Var(hir_name),
                        },
                        span: operation_span.clone(),
                    };
                    let target = self.call_target_for_callee(&callee);
                    HirExprKind::Call(Box::new(callee), vec![left, right], target)
                },
                span: operation_span,
            };
        }

        // Use operator symbol directly as method name (e.g. "+" -> "+", ">>=" -> ">>=")
        let method_name = op_str.clone();

        if matches!(resolved_ty, Type::TypeVar(_)) {
            match self.infer_unresolved_binary_operator_receiver(&left, &right, &method_name) {
                Ok(true) => {
                    resolved_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));
                }
                Ok(false) => {}
                Err(SelectionDiagnostic::AmbiguousCandidates { .. }) => {
                    let result_ty = self.engine.fresh_type_var_at(left.span.clone());
                    let operation_span = left.span.clone();
                    let callee = HirExpr {
                        ty: Type::function(vec![right.ty.clone()], result_ty.clone()),
                        kind: HirExprKind::FieldAccess(Box::new(left), method_name, None),
                        span: operation_span.clone(),
                    };
                    return HirExpr {
                        ty: result_ty,
                        kind: HirExprKind::Call(Box::new(callee), vec![right], None),
                        span: operation_span,
                    };
                }
                Err(error) => {
                    let message = self.display_selection_error(&error);
                    self.diagnostics
                        .push_selection_with_span(message, left.span.clone());
                    return self.error_expression_at(left.span.clone());
                }
            }
        }

        let receiver_candidates = self.receiver_adjustment_candidates(left.clone());
        let found_method = match self.select_operator_method(
            &receiver_candidates,
            &method_name,
            std::slice::from_ref(&right),
        ) {
            Ok(selected) => selected,
            Err(error) => {
                let message = self.display_selection_error(&error);
                self.diagnostics
                    .push_selection_with_span(message, left.span.clone());
                return self.error_expression_at(left.span.clone());
            }
        };

        if let Some(selected) = found_method {
            let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
            let Some((method_func, result_ty, mut coerced_args, target)) =
                self.selected_method_call_types(&selected, vec![right])
            else {
                return self.error_expression_at(left.span.clone());
            };
            let right = coerced_args.remove(0);
            self.report_unsafe_operator_method_call_if_needed(
                &method_func,
                &method_name,
                left.span.clone(),
            );

            return HirExpr {
                ty: result_ty,
                kind: HirExprKind::MethodCall(
                    Box::new(self.apply_receiver_adjustment(
                        selected.receiver,
                        selected.receiver_adjustment,
                    )),
                    method_name,
                    vec![right],
                    method_func.self_receiver,
                    Some(target),
                ),
                span: left.span.clone(),
            };
        }

        let receiver_type = self.display_type(&resolved_ty);
        self.diagnostics.push_selection_with_span(
            format!(
                "No implementation found for operator '{}' on type {}",
                op_str, receiver_type
            ),
            left.span.clone(),
        );

        self.error_expression_at(left.span.clone())
    }

    fn infer_unresolved_binary_operator_receiver(
        &mut self,
        left: &HirExpr,
        right: &HirExpr,
        method_name: &str,
    ) -> Result<bool, SelectionDiagnostic> {
        let right_ty = self.resolve_projection_type(&self.engine.resolve(&right.ty));
        let mut candidates = Vec::new();
        for (_, imp) in self.items.impl_defs_in_order() {
            let Some(crate::hir::HirImplReceiverPattern::Exact(receiver_ty)) =
                Some(&imp.receiver_pattern)
            else {
                continue;
            };
            if !receiver_ty.is_concrete() {
                continue;
            }

            let Some(method) = imp.methods.get(method_name) else {
                continue;
            };
            let param_start = if method.is_method { 1 } else { 0 };
            let Some(arg_param) = method.params.get(param_start) else {
                continue;
            };
            let expected_arg_ty = self.resolve_projection_type(&arg_param.ty);
            let Some(arg_unify_ty) =
                Self::operator_candidate_arg_unify_type(&expected_arg_ty, &right_ty)
            else {
                continue;
            };

            let candidate = (imp.id, receiver_ty.clone(), arg_unify_ty);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }

        let mut candidate_shapes = candidates
            .iter()
            .map(|(_, receiver_ty, arg_ty)| (receiver_ty.clone(), arg_ty.clone()))
            .collect::<Vec<_>>();
        candidate_shapes.sort_by_key(|candidate| format!("{candidate:?}"));
        candidate_shapes.dedup();
        let candidate = if let [candidate] = candidate_shapes.as_slice() {
            candidate
        } else {
            let default = match self.engine.resolve(&left.ty) {
                Type::TypeVar(id) => self
                    .constraint_store
                    .literal_default_type_for_representative(id, |var| {
                        self.engine
                            .unresolved_type_var_representative_for_dependency(var)
                    }),
                _ => None,
            };
            if let Some(default) = default.as_ref() {
                if let Some(candidate) = candidate_shapes
                    .iter()
                    .find(|(receiver_ty, _)| receiver_ty == default)
                {
                    candidate
                } else {
                    return Self::ambiguous_operator_candidates(
                        &self.engine,
                        &left.ty,
                        method_name,
                        &candidates,
                    );
                }
            } else {
                return Self::ambiguous_operator_candidates(
                    &self.engine,
                    &left.ty,
                    method_name,
                    &candidates,
                );
            }
        };
        let (receiver_ty, expected_arg_ty) = candidate;

        Ok(self.engine.unify(&left.ty, receiver_ty).is_ok()
            && self.engine.unify(&right.ty, expected_arg_ty).is_ok())
    }

    fn operator_candidate_arg_unify_type(expected: &Type, actual: &Type) -> Option<Type> {
        if let Type::Reference {
            mutable: false,
            inner,
        } = expected
        {
            if matches!(actual, Type::TypeVar(_)) || inner.as_ref() == actual {
                return Some(inner.as_ref().clone());
            }
        }

        if matches!(actual, Type::TypeVar(_)) || expected == actual {
            return Some(expected.clone());
        }

        None
    }

    fn infer_unresolved_unary_operator_receiver(
        &mut self,
        inner: &HirExpr,
        method_name: &str,
    ) -> Result<bool, SelectionDiagnostic> {
        let mut candidates = Vec::new();
        for (_, imp) in self.items.impl_defs_in_order() {
            let Some(crate::hir::HirImplReceiverPattern::Exact(receiver_ty)) =
                Some(&imp.receiver_pattern)
            else {
                continue;
            };
            if !receiver_ty.is_concrete() {
                continue;
            }

            let Some(method) = imp.methods.get(method_name) else {
                continue;
            };
            let param_start = if method.is_method { 1 } else { 0 };
            if method.params.len() == param_start {
                let candidate = (imp.id, receiver_ty.clone());
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }

        let mut receiver_types = candidates
            .iter()
            .map(|(_, receiver_ty)| receiver_ty.clone())
            .collect::<Vec<_>>();
        receiver_types.sort_by_key(|receiver_ty| format!("{receiver_ty:?}"));
        receiver_types.dedup();
        let receiver_ty = if let [receiver_ty] = receiver_types.as_slice() {
            receiver_ty
        } else {
            let default = match self.engine.resolve(&inner.ty) {
                Type::TypeVar(id) => self
                    .constraint_store
                    .literal_default_type_for_representative(id, |var| {
                        self.engine
                            .unresolved_type_var_representative_for_dependency(var)
                    }),
                _ => None,
            };
            if let Some(default) = default.as_ref() {
                if let Some(receiver_ty) = receiver_types.iter().find(|ty| *ty == default) {
                    receiver_ty
                } else {
                    return Self::ambiguous_unary_operator_candidates(
                        &self.engine,
                        &inner.ty,
                        method_name,
                        &candidates,
                    );
                }
            } else {
                return Self::ambiguous_unary_operator_candidates(
                    &self.engine,
                    &inner.ty,
                    method_name,
                    &candidates,
                );
            }
        };

        Ok(self.engine.unify(&inner.ty, receiver_ty).is_ok())
    }

    fn ambiguous_operator_candidates(
        engine: &crate::infer::InferenceEngine,
        receiver: &Type,
        operation: &str,
        candidates: &[(crate::ids::DefId, Type, Type)],
    ) -> Result<bool, SelectionDiagnostic> {
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut candidate_ids = candidates
            .iter()
            .map(|(impl_id, _, _)| *impl_id)
            .collect::<Vec<_>>();
        candidate_ids.sort();
        candidate_ids.dedup();
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: operation.to_string(),
            receiver: engine.resolve(receiver),
            candidates: candidate_ids,
        })
    }

    fn ambiguous_unary_operator_candidates(
        engine: &crate::infer::InferenceEngine,
        receiver: &Type,
        operation: &str,
        candidates: &[(crate::ids::DefId, Type)],
    ) -> Result<bool, SelectionDiagnostic> {
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut candidate_ids = candidates
            .iter()
            .map(|(impl_id, _)| *impl_id)
            .collect::<Vec<_>>();
        candidate_ids.sort();
        candidate_ids.dedup();
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: operation.to_string(),
            receiver: engine.resolve(receiver),
            candidates: candidate_ids,
        })
    }

    pub(crate) fn select_operator_method(
        &mut self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        args: &[HirExpr],
    ) -> Result<Option<SelectedMethod>, SelectionDiagnostic> {
        let result = self.selection_service().select_concrete_method_matching(
            receiver_candidates,
            method_name,
            |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            |selected| self.operator_method_accepts_args(selected, args),
        );
        match result {
            Ok(selected) => Ok(Some(selected)),
            Err(SelectionDiagnostic::NoImplementation { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn error_expression_at(&self, span: Span) -> HirExpr {
        HirExpr {
            ty: Type::Error,
            kind: HirExprKind::Var("<error>".to_string()),
            span,
        }
    }

    pub(crate) fn lower_unary_expr(&mut self, expr: &ast::UnaryExpr) -> HirExpr {
        self.lower_unary_expr_with_use(expr, ExprUse::Value)
    }

    pub(crate) fn lower_unary_expr_with_use(
        &mut self,
        expr: &ast::UnaryExpr,
        use_kind: ExprUse,
    ) -> HirExpr {
        match expr {
            ast::UnaryExpr::PrimaryExpr(primary) => {
                if use_kind == ExprUse::Value {
                    self.lower_primary_expr(primary)
                } else {
                    self.lower_primary_expr_with_use(primary, use_kind)
                }
            }
            ast::UnaryExpr::UnaryExpr(op, inner) => {
                // Special handling for & / &mut (reference operator)
                if op.value == "&" || op.value == "&mut" {
                    let inner_use = if op.value == "&mut" {
                        ExprUse::AssignmentPlace
                    } else {
                        ExprUse::Value
                    };
                    let inner_hir = self.lower_unary_expr_with_use(inner, inner_use);

                    if op.value == "&mut" {
                        if let HirExprKind::Var(name) = &inner_hir.kind {
                            if let Some(binding) = self.scope.lookup(name) {
                                if !binding.mutable {
                                    self.diagnostics.push_with_span(
                                        format!(
                                            "Cannot take a mutable reference to immutable binding '{}'",
                                            name
                                        ),
                                        op.span.clone(),
                                    );
                                }
                            }
                        }
                    }

                    let ref_ty = Type::Reference {
                        mutable: op.value == "&mut",
                        inner: Box::new(inner_hir.ty.clone()),
                    };
                    return HirExpr {
                        ty: ref_ty,
                        kind: HirExprKind::Ref(op.value == "&mut", Box::new(inner_hir)),
                        span: op.span.clone(),
                    };
                }

                let inner_hir = self.lower_unary_expr_with_use(inner, ExprUse::Value);

                // Special handling for * (dereference operator)
                if op.value == "*" {
                    let resolved_inner_ty = self.engine.resolve(&inner_hir.ty);
                    let inner_span = inner_hir.span.clone();
                    if let Type::Reference { inner, .. } = &resolved_inner_ty {
                        return HirExpr {
                            ty: (*inner.clone()),
                            kind: HirExprKind::Deref(Box::new(inner_hir)),
                            span: inner_span.clone(),
                        };
                    }

                    if let Type::Pointer(inner) = &resolved_inner_ty {
                        if !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                "Dereference of raw pointer requires an unsafe block".to_string(),
                                op.span.clone(),
                            );
                        }

                        return HirExpr {
                            ty: (*inner.clone()),
                            kind: HirExprKind::Deref(Box::new(inner_hir)),
                            span: inner_span,
                        };
                    }

                    if let Some(next) = self
                        .autoderef_candidates(inner_hir.clone())
                        .into_iter()
                        .nth(1)
                    {
                        return next;
                    }

                    let result_ty = resolved_inner_ty.clone();
                    let receiver_type = self.display_type(&resolved_inner_ty);
                    self.diagnostics.push_type_with_span(
                        format!("Cannot dereference non-pointer type: {}", receiver_type),
                        op.span.clone(),
                    );
                    return HirExpr {
                        ty: result_ty,
                        kind: HirExprKind::Deref(Box::new(inner_hir)),
                        span: op.span.clone(),
                    };
                }

                if op.value == "-" || op.value == "!" {
                    let method_name = op.value.as_str();
                    let mut resolved_ty =
                        self.resolve_projection_type(&self.engine.resolve(&inner_hir.ty));

                    if matches!(resolved_ty, Type::TypeVar(_)) {
                        match self.infer_unresolved_unary_operator_receiver(&inner_hir, method_name)
                        {
                            Ok(true) => {
                                resolved_ty = self
                                    .resolve_projection_type(&self.engine.resolve(&inner_hir.ty));
                            }
                            Ok(false) => {}
                            Err(SelectionDiagnostic::AmbiguousCandidates { .. }) => {
                                let result_ty = self.engine.fresh_type_var_at(op.span.clone());
                                let callee = HirExpr {
                                    ty: Type::function(Vec::new(), result_ty.clone()),
                                    kind: HirExprKind::FieldAccess(
                                        Box::new(inner_hir),
                                        method_name.to_string(),
                                        None,
                                    ),
                                    span: op.span.clone(),
                                };
                                return HirExpr {
                                    ty: result_ty,
                                    kind: HirExprKind::Call(Box::new(callee), Vec::new(), None),
                                    span: op.span.clone(),
                                };
                            }
                            Err(error) => {
                                let message = self.display_selection_error(&error);
                                self.diagnostics
                                    .push_selection_with_span(message, op.span.clone());
                                return self.error_expression_at(op.span.clone());
                            }
                        }
                    }

                    let bound_method_result = match &resolved_ty {
                        Type::TypeVar(id) => {
                            let bounds = self.engine.get_bounds(*id);
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(inner_hir.clone());
                            Some(
                                self.selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        method_name,
                                        resolved_ty.clone(),
                                        matches!(&inner_hir.kind, HirExprKind::Call(_, _, _)),
                                    ),
                            )
                        }
                        Type::Generic(gen_param) => {
                            let bounds = self
                                .current_impl_bounds()
                                .get(gen_param)
                                .cloned()
                                .unwrap_or_default();
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(inner_hir.clone());
                            Some(
                                self.selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        method_name,
                                        resolved_ty.clone(),
                                        matches!(&inner_hir.kind, HirExprKind::Call(_, _, _)),
                                    ),
                            )
                        }
                        _ => None,
                    };
                    let bound_method = bound_method_result
                        .and_then(|result| self.handle_optional_selection(result, op.span.clone()));

                    if let Some(selected) = bound_method {
                        let Some((method_func, result_ty, args, target)) =
                            self.selected_method_call_types(&selected, Vec::new())
                        else {
                            return self.error_expression_at(op.span.clone());
                        };
                        self.report_unsafe_operator_method_call_if_needed(
                            &method_func,
                            method_name,
                            inner_hir.span.clone(),
                        );

                        return HirExpr {
                            ty: result_ty,
                            kind: HirExprKind::MethodCall(
                                Box::new(self.apply_receiver_adjustment(
                                    selected.receiver,
                                    selected.receiver_adjustment,
                                )),
                                method_name.to_string(),
                                args,
                                method_func.self_receiver,
                                Some(target),
                            ),
                            span: inner_hir.span.clone(),
                        };
                    }

                    let receiver_candidates =
                        self.receiver_adjustment_candidates(inner_hir.clone());
                    let selected =
                        match self.select_operator_method(&receiver_candidates, method_name, &[]) {
                            Ok(selected) => selected,
                            Err(error) => {
                                let message = self.display_selection_error(&error);
                                self.diagnostics
                                    .push_selection_with_span(message, op.span.clone());
                                return self.error_expression_at(op.span.clone());
                            }
                        };
                    if let Some(selected) = selected {
                        let Some((method_func, result_ty, args, target)) =
                            self.selected_method_call_types(&selected, Vec::new())
                        else {
                            return self.error_expression_at(op.span.clone());
                        };
                        self.report_unsafe_operator_method_call_if_needed(
                            &method_func,
                            method_name,
                            op.span.clone(),
                        );

                        return HirExpr {
                            ty: result_ty,
                            kind: HirExprKind::MethodCall(
                                Box::new(self.apply_receiver_adjustment(
                                    selected.receiver,
                                    selected.receiver_adjustment,
                                )),
                                method_name.to_string(),
                                args,
                                method_func.self_receiver,
                                Some(target),
                            ),
                            span: inner_hir.span.clone(),
                        };
                    }

                    let receiver_type = self.display_type(&resolved_ty);
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "No implementation found for operator '{}' on type {}",
                            op.value, receiver_type
                        ),
                        op.span.clone(),
                    );
                    return self.error_expression_at(op.span.clone());
                }

                let unary_op = match op.value.as_str() {
                    "~" => UnaryOp::BitNot,
                    _ => {
                        self.diagnostics.push_with_span(
                            format!("Unknown unary operator: {}", op.value),
                            op.span.clone(),
                        );
                        UnaryOp::BitNot
                    }
                };
                let ty = inner_hir.ty.clone();
                let inner_span = inner_hir.span.clone();
                HirExpr {
                    ty,
                    kind: HirExprKind::UnaryOp(unary_op, Box::new(inner_hir)),
                    span: inner_span,
                }
            }
        }
    }

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
            ast::Operand::NativeOperator(op) => {
                // Native operator reference
                let ty = self.engine.fresh_type_var_at(op.span.clone());
                HirExpr {
                    ty,
                    kind: HirExprKind::Var(op.name.clone()),
                    span: op.span.clone(),
                }
            }
        }
    }

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
            ast::LiteralKind::Char(c) => HirExpr {
                ty: Type::Char,
                kind: HirExprKind::CharLiteral(c.chars().next().unwrap_or('\0')),
                span,
            },
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
                    ty: Type::Array(Box::new(elem_ty), len),
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
                    ty: Type::Array(Box::new(elem_ty), *len),
                    kind: HirExprKind::ArrayRepeat(Box::new(value), *len),
                    span,
                }
            }
        }
    }

    fn is_fat_raw_slice_pointer(ty: &Type) -> bool {
        matches!(ty, Type::Pointer(inner) if matches!(inner.as_ref(), Type::Slice(_)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::ast::{
        Block, Expression, Ident, IdentOrType, IdentifierPath, Literal, LiteralKind, Operand,
        Operator, PrimaryExpr, Statement, UnaryExpr,
    };
    use crate::hir::{
        HirBlock, HirCallTarget, HirEnum, HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner,
        HirParam, HirStruct, HirVarTarget,
    };
    use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
    use crate::lexer::Span;
    use crate::lower::Lowerer;
    use crate::selection::SelectionDiagnostic;
    use crate::types::{GenericParamDecl, GenericParamId, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn unsafe_control_statements_do_not_require_a_value_span() {
        let mut lowerer = Lowerer::new_for_test();
        for statement in [
            Statement::Continue(None),
            Statement::Return(None),
            Statement::Break(None),
        ] {
            let operand = Operand::Unsafe(
                Block {
                    statements: vec![statement],
                },
                Span::test(),
            );
            let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_operand(&operand));
            assert!(matches!(hir.kind, HirExprKind::UnsafeBlock(_)));
            assert_eq!(hir.span, Span::test());
        }
    }

    #[test]
    fn raw_pointer_dereference_diagnostic_is_selection() {
        let mut lowerer = Lowerer::new_for_test();
        let operator = Operator {
            value: "*".to_string(),
            span: Span::test(),
        };
        let expression = UnaryExpr::UnaryExpr(
            operator,
            Box::new(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident {
                        name: "ptr".to_string(),
                        span: Span::test(),
                    })],
                }),
                secondaries: None,
                type_annotation: None,
            })),
        );

        lowerer.with_test_body_context(|lowerer| {
            lowerer.scope.define_local(
                "ptr".to_string(),
                Type::Pointer(Box::new(Type::I64)),
                false,
                HirLocalId(0),
            );
            lowerer.lower_unary_expr(&expression);
        });

        assert_eq!(
            lowerer.diagnostics.errors()[0].code,
            crate::diagnostic::DiagnosticCode::Selection
        );
    }

    fn test_function(id: DefId, name: &str, params: Vec<Type>, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: Default::default(),
            params: params
                .into_iter()
                .enumerate()
                .map(|(index, ty)| HirParam {
                    name: format!("p{index}"),
                    local_id: crate::ids::HirLocalId(0),
                    ty,
                    mutable: false,
                    is_ref: false,
                })
                .collect(),
            ret_type,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn instantiate_function_type_freshens_bounds_only_parameters() {
        let mut lowerer = Lowerer::new_for_test();
        let owner = def_id(800);
        let source = GenericParamId { owner, index: 0 };
        let item = GenericParamId { owner, index: 1 };
        let mut function = test_function(owner, "visit", vec![Type::Generic(source)], Type::Unit);
        function.generic_params = vec![
            GenericParamDecl::new(source, "C", crate::type_services::kind::Kind::Type),
            GenericParamDecl::new(item, "T", crate::type_services::kind::Kind::Type),
        ];
        function.generic_bounds.insert(
            source,
            vec![crate::types::TraitBound {
                trait_id: def_id(801),
                type_args: vec![Type::Generic(item)],
            }],
        );
        let mut item_variables = Vec::new();
        for _ in 0..2 {
            let Type::Function { params, .. } =
                lowerer.instantiate_function_type(&function, Span::test())
            else {
                panic!("expected instantiated function");
            };
            let Type::TypeVar(source_variable) = params[0] else {
                panic!("expected fresh source variable");
            };
            let bounds = lowerer.engine.get_bounds(source_variable);
            let Type::TypeVar(item_variable) = bounds[0].type_args[0] else {
                panic!("bounds-only parameter was not instantiated");
            };
            item_variables.push(item_variable);
        }
        assert_ne!(item_variables[0], item_variables[1]);
    }

    fn operator_impl(
        impl_id: DefId,
        method_id: DefId,
        receiver_ty: Type,
        operator: &str,
        param_types: Vec<Type>,
    ) -> HirImpl {
        let mut method = test_function(method_id, operator, param_types, Type::I64);
        method.is_method = true;
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("OperatorOwner".to_string()),
            type_name: "OperatorOwner".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(operator.to_string(), method)]),
        }
    }

    fn int_expr(value: u64) -> UnaryExpr {
        UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        })
    }

    fn string_expr(value: &str) -> UnaryExpr {
        UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::String(value.to_string()),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        })
    }

    fn var_expr(name: &str) -> UnaryExpr {
        UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: name.to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: None,
            type_annotation: None,
        })
    }

    #[test]
    fn test_string_literal_lowers_to_borrowed_str() {
        let mut lowerer = Lowerer::new_for_test();
        let hir = lowerer.lower_literal(&Literal {
            kind: LiteralKind::String("hello".to_string()),
            span: Span::test(),
        });

        assert_eq!(
            hir.ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            }
        );
    }

    #[test]
    fn function_valued_ampersand_records_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(3);
        let param_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let function_ty = Type::function(vec![param_ty.clone()], Type::I64);
        lowerer.items.insert_function(test_function(
            function_id,
            "borrowed",
            vec![param_ty],
            Type::I64,
        ));
        lowerer
            .resolver
            .item_paths
            .insert("borrowed".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "borrowed".to_string());
        lowerer
            .scope
            .define_alias("borrowed".to_string(), function_ty, false);
        lowerer.infix_precedence.insert("&".to_string(), 5);

        let expr = Expression::BinopExpr(
            var_expr("borrowed"),
            Operator {
                value: "&".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(int_expr(1))),
        );
        let hir = lowerer.lower_expression(&expr);

        match hir.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
                assert_eq!(id, function_id);
            }
            other => panic!("expected function-valued ampersand call target, got {other:?}"),
        }
    }

    #[test]
    fn direct_function_target_does_not_require_method_payload() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(33);
        let resolved = crate::lower::resolution::LowerResolvedValue {
            name: "function".to_string(),
            ty: Type::I64,
            target: Some(HirVarTarget::Function(function_id)),
            is_alias: false,
            scope_index: None,
            should_instantiate: false,
        };

        assert_eq!(
            lowerer.instantiate_resolved_value_type(&resolved, Span::test()),
            Type::I64
        );
    }

    #[test]
    fn custom_operator_import_alias_lowers_callee_to_function_id() {
        let mut lowerer = Lowerer::new_for_test();
        let operator_id = def_id(1);
        let inc_id = def_id(2);
        let inc_ty = Type::function(vec![Type::I64], Type::I64);
        let operator_ty = Type::function(vec![Type::I64, inc_ty.clone()], Type::I64);

        lowerer.items.insert_function(test_function(
            operator_id,
            "|>",
            vec![Type::I64, inc_ty.clone()],
            Type::I64,
        ));
        lowerer
            .items
            .insert_function(test_function(inc_id, "inc", vec![Type::I64], Type::I64));
        lowerer
            .scope
            .define_alias("|>".to_string(), operator_ty, false);
        lowerer.resolver.insert_import_alias_with_name(
            "|>".to_string(),
            "ops::|>".to_string(),
            operator_id,
        );
        lowerer.infix_precedence.insert("|>".to_string(), 5);

        let expr = Expression::BinopExpr(
            int_expr(41),
            Operator {
                value: "|>".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(var_expr("inc"))),
        );
        let hir = lowerer.lower_expression(&expr);

        let HirExprKind::Call(callee, _, _) = hir.kind else {
            panic!("expected custom operator to lower to a call");
        };
        match callee.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "ops::|>");
                assert_eq!(reference.target, HirVarTarget::Function(operator_id));
            }
            other => panic!("expected resolved operator function target, got {other:?}"),
        }
    }

    #[test]
    fn custom_operator_module_alias_lowers_through_resolver_id() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(30);
        lowerer.items.insert_function(test_function(
            function_id,
            "demo::ops::%%",
            vec![Type::I64, Type::I64],
            Type::I64,
        ));
        lowerer.resolver.insert_module_alias_with_name(
            "%%".to_string(),
            "demo::ops::%%".to_string(),
            function_id,
        );
        lowerer.infix_precedence.insert("%%".to_string(), 5);

        let expr = Expression::BinopExpr(
            int_expr(1),
            Operator {
                value: "%%".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(int_expr(2))),
        );
        let hir = lowerer.lower_expression(&expr);

        let HirExprKind::Call(callee, _, Some(HirCallTarget::Function(id))) = hir.kind else {
            panic!(
                "expected resolver-backed custom operator call, got {:?}",
                hir.kind
            );
        };
        assert_eq!(id, function_id);
        match callee.kind {
            HirExprKind::ResolvedVar(reference) => assert_eq!(reference.name, "demo::ops::%%"),
            other => panic!("expected resolved operator callee, got {other:?}"),
        }
    }

    #[test]
    fn custom_operator_module_alias_shadows_root_operator_with_same_symbol() {
        let mut lowerer = Lowerer::new_for_test();
        let root_id = def_id(31);
        let module_id = def_id(32);
        lowerer.items.insert_function(test_function(
            root_id,
            "%%",
            vec![Type::I64, Type::I64],
            Type::I64,
        ));
        lowerer.items.insert_function(test_function(
            module_id,
            "demo::ops::%%",
            vec![Type::I64, Type::I64],
            Type::I64,
        ));
        lowerer
            .resolver
            .item_paths
            .insert("%%".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "%%".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "%%".to_string(),
            "demo::ops::%%".to_string(),
            module_id,
        );
        lowerer.infix_precedence.insert("%%".to_string(), 5);

        let expr = Expression::BinopExpr(
            int_expr(1),
            Operator {
                value: "%%".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(int_expr(2))),
        );
        let hir = lowerer.lower_expression(&expr);

        let HirExprKind::Call(callee, _, Some(HirCallTarget::Function(id))) = hir.kind else {
            panic!("expected module alias operator call, got {:?}", hir.kind);
        };
        assert_eq!(id, module_id);
        match callee.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "demo::ops::%%");
                assert_eq!(reference.target, HirVarTarget::Function(module_id));
            }
            other => panic!("expected module alias operator callee, got {other:?}"),
        }
    }

    #[test]
    fn custom_operator_scope_local_lowers_callee_to_local_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let operator_local = crate::ids::HirLocalId(7);
        let operator_ty = Type::function(vec![Type::I64, Type::I64], Type::I64);

        lowerer
            .scope
            .define_local("%%".to_string(), operator_ty, false, operator_local);
        lowerer.infix_precedence.insert("%%".to_string(), 5);

        let expr = Expression::BinopExpr(
            int_expr(40),
            Operator {
                value: "%%".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(int_expr(2))),
        );
        let hir = lowerer.lower_expression(&expr);

        let HirExprKind::Call(callee, _, Some(HirCallTarget::Local(target))) = hir.kind else {
            panic!(
                "expected local custom operator call target, got {:?}",
                hir.kind
            );
        };
        assert_eq!(target, operator_local);
        match callee.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "%%");
                assert_eq!(reference.target, HirVarTarget::Local(operator_local));
            }
            other => panic!("expected resolved local operator callee, got {other:?}"),
        }
    }

    #[test]
    fn inherent_impl_custom_operator_call_carries_method_target() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(10);
        let impl_id = def_id(11);
        let method_id = def_id(12);
        let receiver_ty = Type::Struct {
            id: struct_id,
            args: Vec::new(),
        };
        let mut method = test_function(
            method_id,
            "%%",
            vec![receiver_ty.clone(), Type::I64],
            Type::I64,
        );
        method.is_method = true;

        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Box".to_string());
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("%%".to_string(), method)]),
            })
            .unwrap();
        lowerer.infix_precedence.insert("%%".to_string(), 5);

        let left = HirExpr {
            ty: receiver_ty,
            kind: HirExprKind::Var("box".to_string()),
            span: Span::test(),
        };
        let right = HirExpr {
            ty: Type::I64,
            kind: HirExprKind::IntLiteral(2),
            span: Span::test(),
        };
        let hir = lowerer.build_precedence_tree(&[left, right], &["%%".to_string()], 0, 1);

        match hir.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.trait_id(), None);
                assert!(target.trait_args().is_empty());
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected inherent custom operator target, got {other:?}"),
        }
    }

    #[test]
    fn custom_operator_impl_selection_uses_canonical_impl_method() {
        let mut lowerer = Lowerer::new_for_test();
        let option_id = def_id(20);
        let impl_id = def_id(21);
        let method_id = def_id(22);
        let receiver_ty = Type::Enum {
            id: option_id,
            args: vec![Type::I64],
        };
        let mut method = test_function(
            method_id,
            "<&>",
            vec![receiver_ty.clone(), Type::I64],
            Type::I64,
        );
        method.is_method = true;
        lowerer.items.insert_enumeration(HirEnum {
            id: option_id,
            name: "Option".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: option_id,
                    index: 0,
                },
                "T",
            )],
            variants: Vec::new(),
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(option_id, "Option".to_string());
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("<&>".to_string(), method)]),
            })
            .unwrap();
        lowerer.infix_precedence.insert("<&>".to_string(), 5);
        let left_ty = lowerer.engine.fresh_type_var();
        lowerer
            .engine
            .unify(&left_ty, &receiver_ty)
            .expect("test receiver type should resolve to Option");

        let left = HirExpr {
            ty: left_ty,
            kind: HirExprKind::Var("option".to_string()),
            span: Span::test(),
        };
        let right = HirExpr {
            ty: Type::I64,
            kind: HirExprKind::IntLiteral(2),
            span: Span::test(),
        };
        let hir = lowerer.build_precedence_tree(&[left, right], &["<&>".to_string()], 0, 1);

        match hir.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected impl-backed custom operator target, got {other:?}"),
        }
    }

    #[test]
    fn missing_concrete_operator_impl_lowers_to_error_typed_expression() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.infix_precedence.insert("+".to_string(), 6);
        let expr = Expression::BinopExpr(
            string_expr("left"),
            Operator {
                value: "+".to_string(),
                span: Span::test(),
            },
            Box::new(Expression::UnaryExpr(string_expr("right"))),
        );

        let hir = lowerer.lower_expression(&expr);

        assert_eq!(hir.ty, Type::Error);
        assert!(
            !matches!(hir.kind, HirExprKind::Unit),
            "missing operator impl must not recover as executable unit"
        );
        assert!(lowerer.diagnostics.errors().iter().any(|error| error
            .message
            .contains("No implementation found for operator '+'")));
    }

    #[test]
    fn missing_concrete_unary_operator_impl_lowers_to_error_typed_expression() {
        let mut lowerer = Lowerer::new_for_test();
        let expr = UnaryExpr::UnaryExpr(
            Operator {
                value: "-".to_string(),
                span: Span::test(),
            },
            Box::new(string_expr("not numeric")),
        );

        let hir = lowerer.lower_unary_expr(&expr);

        assert_eq!(hir.ty, Type::Error);
        assert!(
            !matches!(hir.kind, HirExprKind::Unit),
            "missing unary operator impl must not recover as executable unit"
        );
        assert!(lowerer.diagnostics.errors().iter().any(|error| error
            .message
            .contains("No implementation found for operator '-'")));
    }

    #[test]
    fn unresolved_binary_operator_reports_all_ambiguous_impl_ids() {
        let mut lowerer = Lowerer::new_for_test();
        let first_impl = def_id(30);
        let second_impl = def_id(31);
        lowerer
            .items
            .insert_impl(operator_impl(
                first_impl,
                def_id(32),
                Type::Bool,
                "%%",
                vec![Type::Bool, Type::I64],
            ))
            .unwrap();
        lowerer
            .items
            .insert_impl(operator_impl(
                second_impl,
                def_id(33),
                Type::Str,
                "%%",
                vec![Type::Str, Type::I64],
            ))
            .unwrap();
        let left_ty = lowerer.engine.fresh_type_var();
        let left = HirExpr {
            ty: left_ty.clone(),
            kind: HirExprKind::Var("left".to_string()),
            span: Span::test(),
        };
        let right = HirExpr {
            ty: Type::I64,
            kind: HirExprKind::IntLiteral(1),
            span: Span::test(),
        };

        assert_eq!(
            lowerer.infer_unresolved_binary_operator_receiver(&left, &right, "%%"),
            Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: "%%".to_string(),
                receiver: left_ty,
                candidates: vec![first_impl, second_impl],
            })
        );
    }

    #[test]
    fn unresolved_unary_operator_reports_all_ambiguous_impl_ids() {
        let mut lowerer = Lowerer::new_for_test();
        let first_impl = def_id(40);
        let second_impl = def_id(41);
        lowerer
            .items
            .insert_impl(operator_impl(
                first_impl,
                def_id(42),
                Type::Bool,
                "!",
                vec![Type::Bool],
            ))
            .unwrap();
        lowerer
            .items
            .insert_impl(operator_impl(
                second_impl,
                def_id(43),
                Type::Str,
                "!",
                vec![Type::Str],
            ))
            .unwrap();
        let operand_ty = lowerer.engine.fresh_type_var();
        let operand = HirExpr {
            ty: operand_ty.clone(),
            kind: HirExprKind::Var("operand".to_string()),
            span: Span::test(),
        };

        assert_eq!(
            lowerer.infer_unresolved_unary_operator_receiver(&operand, "!"),
            Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: "!".to_string(),
                receiver: operand_ty,
                candidates: vec![first_impl, second_impl],
            })
        );
    }

    #[test]
    fn concrete_operator_does_not_select_first_compatible_impl() {
        let mut lowerer = Lowerer::new_for_test();
        let first_impl = def_id(50);
        let second_impl = def_id(51);
        lowerer
            .items
            .insert_impl(operator_impl(
                first_impl,
                def_id(52),
                Type::Bool,
                "%%",
                vec![Type::Bool, Type::I64],
            ))
            .unwrap();
        lowerer
            .items
            .insert_impl(operator_impl(
                second_impl,
                def_id(53),
                Type::Bool,
                "%%",
                vec![Type::Bool, Type::I64],
            ))
            .unwrap();
        lowerer.infix_precedence.insert("%%".to_string(), 5);
        let left = HirExpr {
            ty: Type::Bool,
            kind: HirExprKind::BoolLiteral(true),
            span: Span::test(),
        };
        let right = HirExpr {
            ty: Type::I64,
            kind: HirExprKind::IntLiteral(1),
            span: Span::test(),
        };

        let hir = lowerer.build_precedence_tree(&[left, right], &["%%".to_string()], 0, 1);

        assert_eq!(hir.ty, Type::Error);
        assert!(lowerer.diagnostics.errors().iter().any(|error| {
            error.message.contains("Ambiguous selection for '%%'")
                && error.message.contains("multiple matching implementations")
                && !error.message.contains("DefId")
        }));
    }
}

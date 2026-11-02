//! Expansion of macros called in expression position (`x = %name args`).
//!
//! Each call is replaced by its expansion parsed as one expression and wrapped
//! in parentheses, so it composes like any other operand. Calls produced by
//! an expansion are expanded too, up to the context's depth limit.

use crate::ast::{
    AssignmentLHS, Block, Condition, Else, Expression, FunctionDecl, If, LambdaDecl, LiteralKind,
    Loop, MacroInvoc, Module, NamedFieldsOrTypesList, Operand, PrimaryExpr, SecondaryExpr,
    Statement, StructDeclField, TopLevel, UnaryExpr,
};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::lexer::{Span, Token, TokenType};
use crate::parser;

use super::registry::MacroRegistry;
use super::{
    expand_declarative_tokens, with_expansion_trace, ExpansionId, MacroExpansionContext,
};

/// Expands every expression-position macro call in the items of `module`.
/// Inline `mod` blocks are left to their own expansion pass.
pub(super) fn expand_module_expressions(
    module: &mut Module,
    registry: &MacroRegistry,
    context: &MacroExpansionContext<'_>,
) -> Result<(), Diagnostics> {
    let mut expander = ExpressionExpander {
        registry,
        context,
        diagnostics: Diagnostics::default(),
    };

    for top_level in &mut module.top_levels {
        match top_level {
            TopLevel::FunctionDecl(function) => expander.function(function),
            TopLevel::StructDecl(structure) => expander.fields(&mut structure.fields),
            TopLevel::EnumDecl(enumeration) => {
                for variant in &mut enumeration.variants {
                    if let NamedFieldsOrTypesList::NamedFields(fields) = &mut variant.fields {
                        expander.fields(fields);
                    }
                }
            }
            TopLevel::TraitDecl(trait_decl) => {
                for method in trait_decl.methods.values_mut() {
                    expander.function(method);
                }
            }
            TopLevel::Impl(implementation) => {
                for method in implementation.methods.values_mut() {
                    expander.function(method);
                }
            }
            _ => {}
        }
    }

    expander.diagnostics.return_if_err()
}

struct ExpressionExpander<'r, 'c> {
    registry: &'r MacroRegistry,
    context: &'r MacroExpansionContext<'c>,
    diagnostics: Diagnostics,
}

impl ExpressionExpander<'_, '_> {
    fn function(&mut self, function: &mut FunctionDecl) {
        self.lambda(&mut function.lambda, 0, None);
    }

    fn fields(&mut self, fields: &mut [StructDeclField]) {
        for field in fields {
            if let Some(default) = &mut field.default {
                self.expression(default, 0, None);
            }
        }
    }

    fn lambda(&mut self, lambda: &mut LambdaDecl, depth: usize, parent: Option<ExpansionId>) {
        self.block(&mut lambda.body, depth, parent);
    }

    fn block(&mut self, block: &mut Block, depth: usize, parent: Option<ExpansionId>) {
        for statement in &mut block.statements {
            self.statement(statement, depth, parent);
        }
    }

    fn statement(&mut self, statement: &mut Statement, depth: usize, parent: Option<ExpansionId>) {
        match statement {
            Statement::Assignment(assignment) => {
                if let AssignmentLHS::Expression(target) = &mut assignment.lhs {
                    self.unary(target, depth, parent);
                }
                self.expression(&mut assignment.rhs, depth, parent);
            }
            Statement::Expression(expr) => self.expression(expr, depth, parent),
            Statement::Return(expr) | Statement::Continue(expr) | Statement::Break(expr) => {
                if let Some(expr) = expr {
                    self.expression(expr, depth, parent);
                }
            }
        }
    }

    fn expression(&mut self, expr: &mut Expression, depth: usize, parent: Option<ExpansionId>) {
        match expr {
            Expression::BinopExpr(left, _, right) => {
                self.unary(left, depth, parent);
                self.expression(right, depth, parent);
            }
            Expression::UnaryExpr(unary) => self.unary(unary, depth, parent),
            Expression::CastExpr(inner, _) => self.expression(inner, depth, parent),
            Expression::Range(range) => {
                if let Some(start) = &mut range.start {
                    self.expression(start, depth, parent);
                }
                if let Some(end) = &mut range.end {
                    self.expression(end, depth, parent);
                }
            }
        }
    }

    fn unary(&mut self, unary: &mut UnaryExpr, depth: usize, parent: Option<ExpansionId>) {
        match unary {
            UnaryExpr::PrimaryExpr(primary) => self.primary(primary, depth, parent),
            UnaryExpr::UnaryExpr(_, inner) => self.unary(inner, depth, parent),
        }
    }

    fn primary(&mut self, primary: &mut PrimaryExpr, depth: usize, parent: Option<ExpansionId>) {
        self.operand(&mut primary.operand, depth, parent);

        for secondary in primary.secondaries.iter_mut().flatten() {
            match secondary {
                SecondaryExpr::Arguments(arguments) => {
                    for argument in arguments {
                        self.expression(&mut argument.arg, depth, parent);
                    }
                }
                SecondaryExpr::Indice(index) => self.expression(index, depth, parent),
                SecondaryExpr::Dot(_) | SecondaryExpr::Interogation => {}
            }
        }
    }

    fn operand(&mut self, operand: &mut Operand, depth: usize, parent: Option<ExpansionId>) {
        match operand {
            Operand::MacroInvoc(invocation) => {
                if let Some(expanded) = self.expand(invocation, depth, parent) {
                    *operand = Operand::Expression(Box::new(expanded));
                }
            }
            Operand::Literal(literal) => match &mut literal.kind {
                LiteralKind::Array(array) => {
                    for element in &mut array.elements {
                        self.expression(element, depth, parent);
                    }
                }
                LiteralKind::ArrayRepeat { value, .. } => self.expression(value, depth, parent),
                _ => {}
            },
            Operand::Instance(instance) => {
                for value in instance.fields.values_mut() {
                    self.expression(value, depth, parent);
                }
            }
            Operand::LambdaDecl(lambda) => self.lambda(lambda, depth, parent),
            Operand::Tuple(tuple) => {
                for element in &mut tuple.elements {
                    self.expression(element, depth, parent);
                }
            }
            Operand::If(if_expr) => self.if_expr(if_expr, depth, parent),
            Operand::Match(match_expr) => {
                self.expression(&mut match_expr.expr, depth, parent);
                for arm in &mut match_expr.arms {
                    if let Some(guard) = &mut arm.condition {
                        self.expression(guard, depth, parent);
                    }
                    self.block(&mut arm.body, depth, parent);
                }
            }
            Operand::Loop(loop_expr) => match loop_expr.as_mut() {
                Loop::While(condition, body, _) => {
                    self.condition(condition, depth, parent);
                    self.block(body, depth, parent);
                }
                Loop::For(_, iterable, body, _) => {
                    self.expression(iterable, depth, parent);
                    self.block(body, depth, parent);
                }
                Loop::Loop(body, _) => self.block(body, depth, parent),
            },
            Operand::Unsafe(block, _) => self.block(block, depth, parent),
            Operand::Expression(inner) => self.expression(inner, depth, parent),
            Operand::Ident(_)
            | Operand::CallHole(_)
            | Operand::SelfIdent(_)
            | Operand::NativeOperator(_) => {}
        }
    }

    fn if_expr(&mut self, if_expr: &mut If, depth: usize, parent: Option<ExpansionId>) {
        self.condition(&mut if_expr.condition, depth, parent);
        self.block(&mut if_expr.then, depth, parent);
        match &mut if_expr.else_ {
            Some(Else::If(else_if)) => self.if_expr(else_if, depth, parent),
            Some(Else::Block(block)) => self.block(block, depth, parent),
            None => {}
        }
    }

    fn condition(&mut self, condition: &mut Condition, depth: usize, parent: Option<ExpansionId>) {
        self.expression(&mut condition.expression, depth, parent);
    }

    /// Expands one call and the calls its expansion produces. Returns `None`
    /// after recording a diagnostic.
    fn expand(
        &mut self,
        invocation: &MacroInvoc,
        depth: usize,
        parent: Option<ExpansionId>,
    ) -> Option<Expression> {
        let name = &invocation.name.name;
        let span = invocation.name.span.clone();

        if depth >= self.context.max_depth {
            self.diagnostics
                .push(self.context.depth_exceeded_diagnostic(span));
            return None;
        }

        let Some(macro_decl) = self.registry.declaration(name) else {
            let message = if self.registry.proc_macro(name).is_some() {
                format!("Proc macro '%{name}' cannot be called in expression position")
            } else {
                format!("Unknown macro '%{name}'")
            };
            self.diagnostics
                .push(Diagnostic::new(message, span).with_code(DiagnosticCode::Macro));
            return None;
        };

        let parent = parent.or_else(|| self.context.take_parent_expansion_for_invocation(&span));
        let (body, expansion_id) = match expand_declarative_tokens(
            macro_decl,
            &invocation.args,
            span.clone(),
            parent,
            self.context,
        ) {
            Ok(expanded) => expanded,
            Err(diagnostics) => {
                self.diagnostics.merge(diagnostics);
                return None;
            }
        };

        let tokens = expression_tokens(body.to_tokens(), &span);
        let mut expanded = match parser::parse_expression_tokens(&tokens, self.context.config) {
            Ok(expr) => expr,
            Err(error) => {
                let mut diagnostics = Diagnostics::from(error);
                self.context
                    .map_generated_diagnostics(&mut diagnostics, expansion_id);
                self.diagnostics
                    .merge(with_expansion_trace(diagnostics, self.context, expansion_id));
                return None;
            }
        };

        self.expression(&mut expanded, depth + 1, Some(expansion_id));
        Some(expanded)
    }
}

/// Drops the line framing a template adds around its body (the leading indent
/// and the trailing line breaks) and terminates the tokens with `Eof`.
fn expression_tokens(mut tokens: Vec<Token>, invocation_span: &Span) -> Vec<Token> {
    let is_framing = |token: &Token| {
        matches!(
            token.token_type,
            TokenType::Indent(_) | TokenType::Eol | TokenType::Eof
        )
    };

    while tokens.last().is_some_and(is_framing) {
        tokens.pop();
    }
    let leading = tokens.iter().take_while(|token| is_framing(token)).count();
    tokens.drain(..leading);

    let eof_span = tokens
        .last()
        .map(|token| token.span.clone())
        .unwrap_or_else(|| invocation_span.clone());
    tokens.push(Token {
        token_type: TokenType::Eof,
        span: eof_span,
    });
    tokens
}

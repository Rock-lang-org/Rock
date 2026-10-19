use crate::ast::*;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};

use super::{expand_declarative, generated_parser::GeneratedSyntax, registry::{MacroDefinition, MacroRegistry}, with_expansion_trace, MacroExpansionContext};

/// A.06 establishes ordinary function-body expression substitution. Loaded
/// modules and the remaining declaration-owned expression fields are A.08.
pub(super) fn expand_function_bodies(module: &mut Module, context: &MacroExpansionContext<'_>) -> Result<(), Diagnostics> {
    let registry = MacroRegistry::build(module, context)?;
    let expander = Expressions { registry: &registry, context };
    for item in &mut module.top_levels {
        if let TopLevel::FunctionDecl(function) = item {
            expander.block(&mut function.lambda.body, 0)?;
        }
    }
    Ok(())
}

struct Expressions<'a, 'cfg> {
    registry: &'a MacroRegistry,
    context: &'a MacroExpansionContext<'cfg>,
}

impl Expressions<'_, '_> {
    fn block(&self, block: &mut Block, depth: usize) -> Result<(), Diagnostics> {
        for statement in &mut block.statements {
            match statement {
                Statement::Assignment(assignment) => {
                    if let AssignmentLHS::Expression(value) = &mut assignment.lhs { self.unary(value, depth)?; }
                    self.expression(&mut assignment.rhs, depth)?;
                }
                Statement::Expression(value) => self.expression(value, depth)?,
                Statement::Return(value) | Statement::Continue(value) | Statement::Break(value) => {
                    if let Some(value) = value { self.expression(value, depth)?; }
                }
            }
        }
        Ok(())
    }

    fn expression(&self, expression: &mut Expression, depth: usize) -> Result<(), Diagnostics> {
        match expression {
            Expression::UnaryExpr(value) => self.unary(value, depth)?,
            Expression::BinopExpr(left, _, right) => { self.unary(left, depth)?; self.expression(right, depth)?; }
            Expression::CastExpr(value, _) => self.expression(value, depth)?,
            Expression::Range(range) => {
                if let Some(value) = &mut range.start { self.expression(value, depth)?; }
                if let Some(value) = &mut range.end { self.expression(value, depth)?; }
            }
        }
        Ok(())
    }

    fn unary(&self, unary: &mut UnaryExpr, depth: usize) -> Result<(), Diagnostics> {
        match unary {
            UnaryExpr::UnaryExpr(_, value) => self.unary(value, depth)?,
            UnaryExpr::PrimaryExpr(primary) => {
                self.operand(&mut primary.operand, depth)?;
                if let Some(secondaries) = &mut primary.secondaries {
                    for secondary in secondaries {
                        match secondary {
                            SecondaryExpr::Arguments(arguments) => for argument in arguments { self.expression(&mut argument.arg, depth)?; },
                            SecondaryExpr::Indice(value) => self.expression(value, depth)?,
                            SecondaryExpr::Dot(_) | SecondaryExpr::Interogation => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn operand(&self, operand: &mut Operand, depth: usize) -> Result<(), Diagnostics> {
        match operand {
            Operand::MacroInvoc(invocation) => {
                let failure = |diagnostic: Diagnostic| {
                    let diagnostics = Diagnostics(vec![diagnostic]);
                    invocation.parent.map(|parent| with_expansion_trace(diagnostics.clone(), self.context, parent)).unwrap_or(diagnostics)
                };
                if depth >= self.context.max_depth {
                    return Err(failure(self.context.depth_exceeded_diagnostic(invocation.span.clone())));
                }
                let resolved = self.registry.resolve(&[], &invocation.path_name(), &invocation.span)?;
                let Some(super::registry::MacroEntry { definition: MacroDefinition::Declarative(declaration), .. }) = resolved else {
                    return Err(failure(Diagnostic::new("expression macro expansion is not implemented for unresolved or procedural macro paths".into(), invocation.span.clone())
                        .with_code(DiagnosticCode::Macro)));
                };
                let GeneratedSyntax::Expression(mut generated) = expand_declarative(declaration, invocation, self.context)? else {
                    return Err(failure(Diagnostic::new("expected an expression macro result".into(), invocation.span.clone()).with_code(DiagnosticCode::Macro)));
                };
                // Execute only the emitted tree, never the invocation's input.
                self.expression(&mut generated, depth + 1)?;
                *operand = Operand::Expression(Box::new(generated));
            }
            Operand::Expression(value) => self.expression(value, depth)?,
            Operand::LambdaDecl(lambda) => self.block(&mut lambda.body, depth)?,
            Operand::Unsafe(block, _) => self.block(block, depth)?,
            Operand::Tuple(tuple) => for value in &mut tuple.elements { self.expression(value, depth)?; },
            Operand::Literal(literal) => match &mut literal.kind {
                LiteralKind::Array(array) => for value in &mut array.elements { self.expression(value, depth)?; },
                LiteralKind::ArrayRepeat { value, .. } => self.expression(value, depth)?,
                _ => {}
            },
            Operand::Instance(instance) => {
                let mut names: Vec<_> = instance.fields.keys().cloned().collect();
                names.sort_by(|a, b| a.span.start.cmp(&b.span.start).then(a.name.cmp(&b.name)));
                for name in names {
                    if let Some(value) = instance.fields.get_mut(&name) { self.expression(value, depth)?; }
                }
            }
            Operand::If(value) => self.if_expression(value, depth)?,
            Operand::Match(value) => {
                self.expression(&mut value.expr, depth)?;
                for arm in &mut value.arms {
                    if let Some(condition) = &mut arm.condition { self.expression(condition, depth)?; }
                    self.block(&mut arm.body, depth)?;
                }
            }
            Operand::Loop(value) => match value.as_mut() {
                Loop::While(condition, block, _) => { self.expression(&mut condition.expression, depth)?; self.block(block, depth)?; }
                Loop::For(_, value, block, _) => { self.expression(value, depth)?; self.block(block, depth)?; }
                Loop::Loop(block, _) => self.block(block, depth)?,
            },
            Operand::Ident(_) | Operand::SelfIdent(_) | Operand::CallHole(_) | Operand::NativeOperator(_) => {}
        }
        Ok(())
    }

    fn if_expression(&self, value: &mut If, depth: usize) -> Result<(), Diagnostics> {
        self.expression(&mut value.condition.expression, depth)?;
        self.block(&mut value.then, depth)?;
        match &mut value.else_ {
            Some(Else::If(value)) => self.if_expression(value, depth)?,
            Some(Else::Block(block)) => self.block(block, depth)?,
            None => {}
        }
        Ok(())
    }
}

use crate::ast::*;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};

use super::{expand_declarative, generated_parser::GeneratedSyntax, registry::{MacroDefinition, MacroRegistry}, with_expansion_trace, MacroExpansionContext};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Items,
    Expressions,
}

/// One scope-aware walk owns all executable AST fields. Macro definitions and
/// invocation inputs are data; only selected emitted syntax is traversed.
pub(super) struct ExpansionVisitor<'a, 'cfg> {
    registry: &'a MacroRegistry,
    context: &'a MacroExpansionContext<'cfg>,
    phase: Phase,
    scope: Vec<String>,
    pub(super) executed: usize,
    pub(super) deferred: Option<Diagnostics>,
}

impl<'a, 'cfg> ExpansionVisitor<'a, 'cfg> {
    pub(super) fn new(registry: &'a MacroRegistry, context: &'a MacroExpansionContext<'cfg>, phase: Phase) -> Self {
        Self { registry, context, phase, scope: Vec::new(), executed: 0, deferred: None }
    }

    pub(super) fn set_scope(&mut self, qualified_name: &str) {
        self.scope = qualified_name.split("::").map(String::from).collect();
        if self.scope.first().map(String::as_str) == self.context.config.current_crate_name.as_deref() {
            self.scope.remove(0);
        }
    }

    pub(super) fn module(&mut self, module: &mut Module) -> Result<(), Diagnostics> {
        let mut output = Vec::new();
        for mut item in std::mem::take(&mut module.top_levels) {
            if self.phase == Phase::Items {
                if let TopLevel::MacroInvoc(invocation) = &item {
                    match self.registry.resolve(&self.scope, &invocation.path_name(), &invocation.span) {
                        Ok(Some(super::registry::MacroEntry { definition: MacroDefinition::Declarative(declaration), .. })) => {
                            let emitted = super::expand_top_level(declaration, invocation, self.context)?;
                            self.context.note_source_modules(&emitted, &self.scope, invocation.id);
                            output.extend(emitted);
                            self.executed += 1;
                            continue;
                        }
                        Ok(Some(super::registry::MacroEntry { definition: MacroDefinition::Procedural(entry), .. })) => {
                            let emitted = super::expand_proc_macro_top_level(entry, invocation, self.context)?;
                            self.context.note_source_modules(&emitted, &self.scope, invocation.id);
                            output.extend(emitted);
                            self.executed += 1;
                            continue;
                        }
                        Err(diagnostics) if self.deferred.is_none() => self.deferred = Some(diagnostics),
                        Ok(None) | Err(_) => {}
                    }
                }
            }
            match &mut item {
                TopLevel::Module(child) => {
                    let old_scope = self.scope.clone();
                    if let Some(name) = &child.0.name { self.scope.push(name.name.clone()); }
                    self.module(&mut child.0)?;
                    self.scope = old_scope;
                }
                TopLevel::FunctionDecl(function) if self.phase == Phase::Expressions => self.lambda(&mut function.lambda, 0)?,
                TopLevel::StructDecl(declaration) if self.phase == Phase::Expressions => self.fields(&mut declaration.fields)?,
                TopLevel::EnumDecl(declaration) if self.phase == Phase::Expressions => {
                    for variant in &mut declaration.variants {
                        if let NamedFieldsOrTypesList::NamedFields(fields) = &mut variant.fields { self.fields(fields)?; }
                    }
                }
                TopLevel::TraitDecl(declaration) if self.phase == Phase::Expressions => self.methods(&mut declaration.methods)?,
                TopLevel::Impl(declaration) if self.phase == Phase::Expressions => self.methods(&mut declaration.methods)?,
                TopLevel::MacroDecl(_) | TopLevel::MacroInvoc(_) | TopLevel::Mod(_, _) | TopLevel::Import(_)
                | TopLevel::GlobImport(_) | TopLevel::Export(_) | TopLevel::GlobExport(_) | TopLevel::Extern(_)
                | TopLevel::InfixOperator(_, _) | TopLevel::FunctionSig(_) | TopLevel::NewType(_, _, _)
                | TopLevel::FunctionDecl(_) | TopLevel::StructDecl(_) | TopLevel::EnumDecl(_)
                | TopLevel::TraitDecl(_) | TopLevel::Impl(_) => {}
            }
            output.push(item);
        }
        module.top_levels = output;
        Ok(())
    }

    fn fields(&self, fields: &mut [StructDeclField]) -> Result<(), Diagnostics> {
        for field in fields {
            if let Some(value) = &mut field.default { self.expression(value, 0)?; }
        }
        Ok(())
    }

    fn methods(&self, methods: &mut std::collections::HashMap<Ident, FunctionDecl>) -> Result<(), Diagnostics> {
        let mut names: Vec<_> = methods.keys().cloned().collect();
        names.sort_by(|a, b| a.span.start.cmp(&b.span.start).then(a.name.cmp(&b.name)));
        for name in names {
            if let Some(function) = methods.get_mut(&name) { self.lambda(&mut function.lambda, 0)?; }
        }
        Ok(())
    }

    fn lambda(&self, lambda: &mut LambdaDecl, depth: usize) -> Result<(), Diagnostics> {
        for parameter in &mut lambda.parameters { self.pattern(parameter, depth)?; }
        self.block(&mut lambda.body, depth)
    }

    fn pattern(&self, pattern: &mut Pattern, depth: usize) -> Result<(), Diagnostics> {
        match &mut pattern.kind {
            PatternKind::Literal(literal) => self.literal(literal, depth)?,
            PatternKind::Tuple(patterns) => for pattern in patterns { self.pattern(pattern, depth)?; },
            PatternKind::Array(patterns) => for pattern in patterns {
                if let ArrayPattern::Pattern(pattern) = pattern { self.pattern(pattern, depth)?; }
            },
            PatternKind::Instance(instance) => match &mut instance.args {
                FieldsPatternOrArgumentsPattern::Fields(fields) => for field in fields { self.pattern(&mut field.pattern, depth)?; },
                FieldsPatternOrArgumentsPattern::Arguments(patterns) => for pattern in patterns { self.pattern(pattern, depth)?; },
            },
            PatternKind::Nested(pattern) | PatternKind::Reference { pattern, .. } => self.pattern(pattern, depth)?,
            PatternKind::Ident(_) | PatternKind::Wildcard => {}
        }
        Ok(())
    }

    fn block(&self, block: &mut Block, depth: usize) -> Result<(), Diagnostics> {
        for statement in &mut block.statements {
            match statement {
                Statement::Assignment(assignment) => {
                    match &mut assignment.lhs {
                        AssignmentLHS::Expression(value) => self.unary(value, depth)?,
                        AssignmentLHS::Pattern { pattern, .. } => self.pattern(pattern, depth)?,
                    }
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
                let resolved = self.registry.resolve(&self.scope, &invocation.path_name(), &invocation.span)?;
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
            Operand::LambdaDecl(lambda) => self.lambda(lambda, depth)?,
            Operand::Unsafe(block, _) => self.block(block, depth)?,
            Operand::Tuple(tuple) => for value in &mut tuple.elements { self.expression(value, depth)?; },
            Operand::Literal(literal) => self.literal(literal, depth)?,
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
                    self.pattern(&mut arm.pattern, depth)?;
                    if let Some(condition) = &mut arm.condition { self.expression(condition, depth)?; }
                    self.block(&mut arm.body, depth)?;
                }
            }
            Operand::Loop(value) => match value.as_mut() {
                Loop::While(condition, block, _) => { self.condition(condition, depth)?; self.block(block, depth)?; }
                Loop::For(pattern, value, block, _) => { self.pattern(pattern, depth)?; self.expression(value, depth)?; self.block(block, depth)?; }
                Loop::Loop(block, _) => self.block(block, depth)?,
            },
            Operand::Ident(_) | Operand::SelfIdent(_) | Operand::CallHole(_) | Operand::NativeOperator(_) => {}
        }
        Ok(())
    }

    fn if_expression(&self, value: &mut If, depth: usize) -> Result<(), Diagnostics> {
        self.condition(&mut value.condition, depth)?;
        self.block(&mut value.then, depth)?;
        match &mut value.else_ {
            Some(Else::If(value)) => self.if_expression(value, depth)?,
            Some(Else::Block(block)) => self.block(block, depth)?,
            None => {}
        }
        Ok(())
    }

    fn condition(&self, condition: &mut Condition, depth: usize) -> Result<(), Diagnostics> {
        if let Some(pattern) = &mut condition.pattern { self.pattern(pattern, depth)?; }
        self.expression(&mut condition.expression, depth)
    }

    fn literal(&self, literal: &mut Literal, depth: usize) -> Result<(), Diagnostics> {
        match &mut literal.kind {
            LiteralKind::Array(array) => for value in &mut array.elements { self.expression(value, depth)?; },
            LiteralKind::ArrayRepeat { value, .. } => self.expression(value, depth)?,
            LiteralKind::Bool(_) | LiteralKind::Number(_) | LiteralKind::Float(_) | LiteralKind::String(_) | LiteralKind::Char(_) => {}
        }
        Ok(())
    }
}

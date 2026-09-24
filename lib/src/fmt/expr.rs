use std::fmt::{self, Write};

use crate::ast::*;

use super::decl::display_block;
use super::{FormatContext, FormatNode};

impl FormatNode for Statement {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Statement::Assignment(assign) => assign.fmt_with(context, f),
            Statement::Expression(expr) => expr.fmt_with(context, f),
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    write!(f, "return ")?;
                    expr.fmt_with(context, f)
                } else {
                    write!(f, "return")
                }
            }
            Statement::Continue(expr) => {
                if let Some(expr) = expr {
                    write!(f, "continue ")?;
                    expr.fmt_with(context, f)
                } else {
                    write!(f, "continue")
                }
            }
            Statement::Break(expr) => {
                if let Some(expr) = expr {
                    write!(f, "break ")?;
                    expr.fmt_with(context, f)
                } else {
                    write!(f, "break")
                }
            }
        }
    }
}

impl FormatNode for Assignment {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.lhs.fmt_with(context, f)?;
        write!(f, " = ")?;
        self.rhs.fmt_with(context, f)
    }
}

impl FormatNode for AssignmentLHS {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            AssignmentLHS::Expression(expr) => expr.fmt_with(context, f),
            AssignmentLHS::Pattern {
                pattern,
                type_annotation,
            } => {
                pattern.fmt_with(context, f)?;

                if let Some(type_annotation) = type_annotation {
                    write!(f, ": ")?;
                    type_annotation.fmt_with(context, f)?;
                }

                Ok(())
            }
        }
    }
}

impl FormatNode for Expression {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Expression::BinopExpr(lhs, op, rhs) => {
                lhs.fmt_with(context, f)?;
                write!(f, " ")?;
                op.fmt_with(context, f)?;
                write!(f, " ")?;
                rhs.fmt_with(context, f)
            }
            Expression::UnaryExpr(expr) => expr.fmt_with(context, f),
            Expression::CastExpr(expr, ty) => {
                expr.fmt_with(context, f)?;
                write!(f, " as ")?;
                ty.fmt_with(context, f)
            }
            Expression::Range(range) => {
                if let Some(start) = &range.start {
                    start.fmt_with(context, f)?;
                }
                write!(f, "{}", if range.inclusive { "..=" } else { ".." })?;
                if let Some(end) = &range.end {
                    end.fmt_with(context, f)?;
                }
                Ok(())
            }
        }
    }
}

impl FormatNode for UnaryExpr {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            UnaryExpr::PrimaryExpr(expr) => expr.fmt_with(context, f),
            UnaryExpr::UnaryExpr(op, expr) => {
                op.fmt_with(context, f)?;
                expr.fmt_with(context, f)
            }
        }
    }
}

impl FormatNode for Operator {
    fn fmt_with<W: Write>(&self, _context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl FormatNode for PrimaryExpr {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.operand.fmt_with(context, f)?;

        if let Some(secondaries) = &self.secondaries {
            for (i, secondary) in secondaries.iter().enumerate() {
                secondary.fmt_with(context, f)?;

                if let SecondaryExpr::Arguments(_) = secondary {
                    if i < secondaries.len() - 1 {
                        if let SecondaryExpr::Dot(_) = secondaries[i + 1] {
                            write!(f, " ")?;
                            continue;
                        }
                    }
                }
            }
        }

        if let Some(type_annotation) = &self.type_annotation {
            write!(f, " : ")?;
            type_annotation.fmt_with(context, f)?;
        }

        Ok(())
    }
}

impl FormatNode for Operand {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Operand::Literal(lit) => lit.fmt_with(context, f),
            Operand::Ident(ident) => ident.fmt_with(context, f),
            Operand::CallHole(_) => write!(f, "_"),
            Operand::SelfIdent(ident) => ident.fmt_with(context, f),
            Operand::Instance(inst) => inst.fmt_with(context, f),
            Operand::NativeOperator(op) => op.fmt_with(context, f),
            Operand::LambdaDecl(decl) => decl.fmt_with(context, f),
            Operand::Tuple(tuple) => tuple.fmt_with(context, f),
            Operand::If(if_) => if_.fmt_with(context, f),
            Operand::Match(match_) => match_.fmt_with(context, f),
            Operand::Loop(loop_) => loop_.fmt_with(context, f),
            Operand::Expression(expr) => {
                write!(f, "(")?;
                expr.fmt_with(context, f)?;
                write!(f, ")")
            }
            Operand::Unsafe(block, _) => {
                write!(f, "unsafe")?;
                display_block(context, block, true, f)
            }
        }
    }
}

impl FormatNode for Match {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(statements) = &self.do_syntax {
            writeln!(f, "do")?;
            context.increase_indent();
            for (index, statement) in statements.iter().enumerate() {
                context.write_indent(f)?;
                match statement {
                    DoStatement::Bind(pattern, expression, _) => {
                        pattern.fmt_with(context, f)?;
                        write!(f, " <- ")?;
                        expression.fmt_with(context, f)?;
                    }
                    DoStatement::Statement(statement, _) => statement.fmt_with(context, f)?,
                }
                if index + 1 < statements.len() {
                    writeln!(f)?;
                }
            }
            context.decrease_indent();
            return Ok(());
        }
        write!(f, "match ")?;
        self.expr.fmt_with(context, f)?;
        writeln!(f)?;

        context.increase_indent();
        for (i, arm) in self.arms.iter().enumerate() {
            context.write_indent(f)?;
            arm.fmt_with(context, f)?;

            if i < self.arms.len() - 1 {
                writeln!(f)?;
            }
        }
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for MatchArm {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.pattern.fmt_with(context, f)?;
        if let Some(condition) = &self.condition {
            write!(f, " if ")?;
            condition.fmt_with(context, f)?;
        }
        write!(f, " => ")?;
        display_block(context, &self.body, false, f)
    }
}

impl FormatNode for Tuple {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "(")?;

        for (i, expr) in self.elements.iter().enumerate() {
            expr.fmt_with(context, f)?;

            if i < self.elements.len() - 1 {
                write!(f, ", ")?;
            }
        }

        write!(f, ")")
    }
}

impl FormatNode for NativeOperator {
    fn fmt_with<W: Write>(&self, _context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "~{}", self.name)
    }
}

impl FormatNode for SecondaryExpr {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            SecondaryExpr::Arguments(args) => {
                if args.is_empty() {
                    write!(f, "!")?;
                    return Ok(());
                }

                for (i, arg) in args.iter().enumerate() {
                    write!(f, " ")?;
                    arg.fmt_with(context, f)?;

                    if i < args.len() - 1 {
                        write!(f, ",")?;
                    }
                }

                Ok(())
            }
            SecondaryExpr::Indice(indice) => {
                write!(f, "[")?;
                indice.fmt_with(context, f)?;
                write!(f, "]")
            }
            SecondaryExpr::Dot(field) => {
                write!(f, ".")?;
                field.fmt_with(context, f)
            }
            SecondaryExpr::Interogation => write!(f, "?"),
        }
    }
}

impl FormatNode for IdentOrNumber {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            IdentOrNumber::Ident(ident) => ident.fmt_with(context, f),
            IdentOrNumber::Number(num) => write!(f, "{}", num),
        }
    }
}

impl FormatNode for Argument {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.arg.fmt_with(context, f)
    }
}

impl FormatNode for Literal {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match &self.kind {
            LiteralKind::Bool(b) => write!(f, "{}", b),
            LiteralKind::Number(num) => write!(f, "{}", num),
            LiteralKind::Float(num) => write!(f, "{}", num),
            LiteralKind::Array(s) => s.fmt_with(context, f),
            LiteralKind::ArrayRepeat { value, len } => {
                write!(f, "[")?;
                value.fmt_with(context, f)?;
                write!(f, "; {}]", len)
            }
            LiteralKind::String(s) => write!(f, "\"{}\"", s),
            LiteralKind::Char(c) => write!(f, "'{}'", c),
        }
    }
}

impl FormatNode for Instance {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.name.fmt_with(context, f)?;

        context.increase_indent();

        if !self.fields.is_empty() {
            writeln!(f)?;
        }

        for (i, (field, value)) in self.fields.iter().enumerate() {
            context.write_indent(f)?;
            field.fmt_with(context, f)?;
            write!(f, ": ")?;
            value.fmt_with(context, f)?;

            if i < self.fields.len() - 1 {
                writeln!(f)?;
            }
        }

        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for Condition {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(pattern) = &self.pattern {
            pattern.fmt_with(context, f)?;
            write!(f, " = ")?;
        }

        self.expression.fmt_with(context, f)
    }
}

impl FormatNode for If {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "if ")?;
        self.condition.fmt_with(context, f)?;
        writeln!(f)?;
        context.write_indent(f)?;
        write!(f, "then")?;

        if self.then.statements.len() <= 1 {
            write!(f, " ")?;
        }

        display_block(context, &self.then, false, f)?;

        if let Some(else_) = &self.else_ {
            writeln!(f)?;
            context.write_indent(f)?;
            write!(f, "else")?;
            else_.fmt_with(context, f)
        } else {
            Ok(())
        }
    }
}

impl FormatNode for Else {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Else::If(if_) => if_.fmt_with(context, f),
            Else::Block(block) => {
                if block.statements.len() <= 1 {
                    write!(f, " ")?;
                }
                display_block(context, block, false, f)
            }
        }
    }
}

impl FormatNode for Loop {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Loop::While(cond, block, _) => {
                write!(f, "while ")?;
                cond.fmt_with(context, f)?;
                display_block(context, block, true, f)
            }
            Loop::For(ident, cond, block, _) => {
                write!(f, "for ")?;
                ident.fmt_with(context, f)?;
                write!(f, " in ")?;
                cond.fmt_with(context, f)?;
                display_block(context, block, true, f)
            }
            Loop::Loop(block, _) => {
                write!(f, "loop")?;
                display_block(context, block, true, f)
            }
        }
    }
}

impl FormatNode for Array {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "[")?;

        for (i, expr) in self.elements.iter().enumerate() {
            expr.fmt_with(context, f)?;

            if i < self.elements.len() - 1 {
                write!(f, ", ")?;
            }
        }

        write!(f, "]")
    }
}

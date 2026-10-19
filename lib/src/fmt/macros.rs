use std::fmt::{self, Write};

use crate::{ast::*, lexer::TokenType};

use super::{FormatContext, FormatNode};

impl FormatNode for MacroDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "macro ")?;
        self.name.fmt_with(context, f)?;
        writeln!(f)?;

        for entry in &self.entries {
            entry.fmt_with(context, f)?;
        }

        Ok(())
    }
}

impl FormatNode for MacroEntry {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        context.increase_indent();

        context.write_indent(f)?;
        for def in &self.defs {
            HeadMacroFragment(def.clone()).fmt_with(context, f)?;
            write!(f, " ")?;
        }

        writeln!(f, "=>")?;

        context.increase_indent();
        for (i, fragment) in self.body.iter().enumerate() {
            let next_is_eol = self.body.get(i + 1).map_or(true, |f| {
                if let MacroFragment::Token(token) = f {
                    token.token_type == TokenType::Eol
                } else {
                    false
                }
            });

            let write_space = !next_is_eol && i < self.body.len() - 1;

            if let MacroFragment::Token(token) = fragment {
                if let TokenType::Indent(_) = token.token_type {
                    fragment.fmt_with(context, f)?;
                } else if let TokenType::StuckOperator(_) = token.token_type {
                    fragment.fmt_with(context, f)?;
                } else {
                    fragment.fmt_with(context, f)?;

                    if write_space && token.token_type != TokenType::Eol {
                        write!(f, " ")?;
                    }
                }
            } else {
                fragment.fmt_with(context, f)?;

                if write_space {
                    write!(f, " ")?;
                }
            }
        }

        context.decrease_indent();
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for MacroFragment {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            MacroFragment::Ident(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)
            }
            MacroFragment::Expr(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)
            }
            MacroFragment::Type(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)
            }
            MacroFragment::Token(token) => {
                if let TokenType::Indent(_) = token.token_type {
                    context.write_indent(f)?;
                }
                write!(f, "{}", token)
            }
            MacroFragment::Trivia(input) => {
                for run in input.runs() { f.write_str(run.text())?; }
                Ok(())
            }
            MacroFragment::Repetition(fragments) => {
                write!(f, "$(")?;

                for fragment in fragments {
                    fragment.fmt_with(context, f)?;
                }

                write!(f, ")*")
            }
        }
    }
}

struct HeadMacroFragment(MacroFragment);

impl HeadMacroFragment {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match &self.0 {
            MacroFragment::Ident(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)?;
                write!(f, ":ident")
            }
            MacroFragment::Expr(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)?;
                write!(f, ":expr")
            }
            MacroFragment::Type(ident) => {
                write!(f, "$")?;
                ident.fmt_with(context, f)?;
                write!(f, ":ty")
            }
            MacroFragment::Token(token) => write!(f, "{}", token),
            MacroFragment::Trivia(input) => {
                for run in input.runs() { f.write_str(run.text())?; }
                Ok(())
            }
            MacroFragment::Repetition(fragments) => {
                write!(f, "$(")?;

                for fragment in fragments {
                    fragment.fmt_with(context, f)?;

                    if let MacroFragment::Ident(_) = fragment {
                        write!(f, ":ident")?;
                    }
                }

                write!(f, ")*")
            }
        }
    }
}

impl FormatNode for MacroInvoc {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "%")?;
        self.path.fmt_with(context, f)?;

        if self.output == crate::ast::MacroOutputCategory::Expression {
            if self.arguments.is_empty() { return write!(f, "!"); }
            for run in self.args.runs() { f.write_str(run.text())?; }
            return Ok(());
        }
        let mut view = crate::lexer::interpretation::layout::LayoutView::new(&self.args, false);
        while let Some(piece) = view.next_token().map_err(|_| fmt::Error)? {
            if !matches!(piece.token.token_type, TokenType::Indent(_) | TokenType::Eol | TokenType::Eof) {
                write!(f, " {}", piece.token)?;
            }
        }

        Ok(())
    }
}

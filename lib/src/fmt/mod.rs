mod decl;
mod expr;
mod macros;
mod pattern;
mod trivia;

use std::fmt::{self, Write};

use crate::ast::*;

pub use trivia::FormatTrivia;

#[derive(Debug, Clone, Default)]
pub struct FormatContext {
    indent: usize,
    inside_fn_type_decl: bool,
}

/// Formatter-facing input document.
///
/// Construct this from parsed syntax instead of formatting AST nodes directly.
/// The fields stay private so source trivia side tables remain formatter-only.
#[derive(Debug, Clone)]
pub struct FormatInput<'a> {
    document: FormatDocument<'a>,
    trivia: Option<FormatTrivia<'a>>,
}

#[derive(Debug, Clone, Copy)]
enum FormatDocument<'a> {
    Program(&'a Program),
    Module(&'a Module),
}

impl<'a> FormatInput<'a> {
    /// Build formatter input for a parsed program.
    pub fn program(program: &'a Program) -> Self {
        Self {
            document: FormatDocument::Program(program),
            trivia: None,
        }
    }

    /// Build formatter input for a parsed module.
    pub fn module(module: &'a Module) -> Self {
        Self {
            document: FormatDocument::Module(module),
            trivia: None,
        }
    }

    /// Build formatter input for a parsed module and its original source text.
    pub fn module_with_source(module: &'a Module, source: &'a str) -> Self {
        Self {
            document: FormatDocument::Module(module),
            trivia: Some(FormatTrivia::from_module_source(module, source)),
        }
    }
}

/// Format a formatter-facing input document.
pub fn format(input: FormatInput<'_>) -> String {
    FormatContext::new().format(input)
}

impl FormatContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format(&mut self, input: FormatInput<'_>) -> String {
        match (input.document, input.trivia.as_ref()) {
            (FormatDocument::Program(program), _) => self.format_node(program),
            (FormatDocument::Module(module), Some(trivia)) => {
                self.format_module_with_trivia(module, trivia)
            }
            (FormatDocument::Module(module), None) => self.format_node(module),
        }
    }

    fn format_node<T: FormatNode + ?Sized>(&mut self, node: &T) -> String {
        let mut output = String::new();
        node.fmt_with(self, &mut output)
            .expect("formatting into a String should not fail");
        output
    }

    fn format_module_with_trivia(&mut self, module: &Module, trivia: &FormatTrivia<'_>) -> String {
        let mut output = String::new();

        if !module.is_inline {
            self.increase_indent();
        }

        for (index, top_level) in module.top_levels.iter().enumerate() {
            let mut formatted_top_level = String::new();
            self.write_indent(&mut formatted_top_level)
                .expect("formatting into a String should not fail");
            top_level
                .fmt_with(self, &mut formatted_top_level)
                .expect("formatting into a String should not fail");

            trivia.write_leading(index, &mut output);
            output.push_str(&trivia.apply_to_item(index, &formatted_top_level));
        }

        if !module.is_inline {
            self.decrease_indent();
        }

        trivia.write_module_trailing(&mut output);
        output
    }

    fn write_indent<W: Write>(&self, f: &mut W) -> fmt::Result {
        for _ in 0..self.indent {
            f.write_char(' ')?;
        }
        Ok(())
    }

    fn increase_indent(&mut self) {
        self.indent += 4;
    }

    fn decrease_indent(&mut self) {
        assert!(self.indent >= 4, "formatter indent underflow");
        self.indent -= 4;
    }
}

pub(crate) trait FormatNode {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result;
}

fn write_language_item_member_marker<W: Write>(
    language_items: &LanguageItemAnnotations,
    kind: LanguageItemMemberKind,
    member_name: &str,
    emitted_markers: &mut [bool],
    context: &FormatContext,
    f: &mut W,
) -> fmt::Result {
    let marker_index = language_items
        .members
        .iter()
        .enumerate()
        .position(|(index, marker)| {
            !emitted_markers[index] && marker.kind == kind && marker.member_name == member_name
        });
    if let Some(marker_index) = marker_index {
        emitted_markers[marker_index] = true;
        context.write_indent(f)?;
        writeln!(
            f,
            "lang {}",
            language_items.members[marker_index].marker.role
        )?;
    }

    Ok(())
}

impl FormatNode for Program {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.module.fmt_with(context, f)
    }
}

impl FormatNode for ModuleDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(name) = &self.0.name {
            write!(f, "mod ")?;
            name.fmt_with(context, f)?;
            writeln!(f)?;
        }

        Ok(())
    }
}

impl FormatNode for Module {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if !self.is_inline {
            context.increase_indent();
        }

        for top_level in self.top_levels.iter() {
            context.write_indent(f)?;
            top_level.fmt_with(context, f)?;
        }

        if !self.is_inline {
            context.decrease_indent();
        }

        Ok(())
    }
}

impl FormatNode for TopLevel {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match &self {
            TopLevel::Module(module) => module.fmt_with(context, f),
            TopLevel::Mod(ident, exported) => {
                writeln!(f, "{} mod {}", if *exported { "<" } else { "" }, ident.name)
            }
            TopLevel::InfixOperator(precedence, decl) => {
                writeln!(f, "infix {} {}", precedence, decl)
            }
            TopLevel::Import(path) => {
                write!(f, "> ")?;
                path.fmt_with(context, f)?;
                writeln!(f)
            }
            TopLevel::GlobImport(segs) => writeln!(f, "> {}::*", segs.join("::")),
            TopLevel::Export(path) => {
                write!(f, "< ")?;
                path.fmt_with(context, f)?;
                writeln!(f)
            }
            TopLevel::MacroDecl(decl) => decl.fmt_with(context, f),
            TopLevel::MacroInvoc(invoc) => {
                invoc.fmt_with(context, f)?;
                writeln!(f)
            }
            TopLevel::Extern(sig) => {
                write!(f, "extern ")?;
                sig.fmt_with(context, f)
            }
            TopLevel::FunctionSig(sig) => sig.fmt_with(context, f),
            TopLevel::FunctionDecl(decl) => decl.fmt_with(context, f),
            TopLevel::StructDecl(decl) => decl.fmt_with(context, f),
            TopLevel::TraitDecl(decl) => decl.fmt_with(context, f),
            TopLevel::EnumDecl(decl) => decl.fmt_with(context, f),
            TopLevel::Impl(impl_) => impl_.fmt_with(context, f),
            TopLevel::NewType(inner, params, ty) => {
                write!(f, "type ")?;
                inner.fmt_with(context, f)?;
                for (index, param) in params.iter().enumerate() {
                    write!(f, " ")?;
                    if param.kind.is_some() && param.default.is_none() {
                        write!(f, "(")?;
                    }
                    param.fmt_with(context, f)?;
                    if param.kind.is_some() && param.default.is_none() {
                        write!(f, ")")?;
                    }
                    if index + 1 < params.len() {
                        write!(f, ",")?;
                    }
                }
                write!(f, " = ")?;
                ty.fmt_with(context, f)?;
                writeln!(f)
            }
            TopLevel::GlobExport(segs) => writeln!(f, "< {}::*", segs.join("::")),
        }
    }
}

impl FormatNode for StructDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "struct ")?;
        self.name.fmt_with(context, f)?;
        for (index, param) in self.generic_params.iter().enumerate() {
            write!(f, " ")?;
            if param.kind.is_some() && param.default.is_none() {
                write!(f, "(")?;
                param.fmt_with(context, f)?;
                write!(f, ")")?;
            } else {
                param.fmt_with(context, f)?;
            }
            if index + 1 < self.generic_params.len() {
                write!(f, ",")?;
            }
        }
        writeln!(f)?;

        context.increase_indent();
        for field in &self.fields {
            context.write_indent(f)?;
            field.fmt_with(context, f)?;
            writeln!(f)?;
        }
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for StructDeclField {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        let public_str = if self.public { "< " } else { "" };
        write!(f, "{}", public_str)?;
        self.name.fmt_with(context, f)?;
        write!(f, " : ")?;
        self.ty.fmt_with(context, f)
    }
}

impl FormatNode for EnumDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(marker) = &self.language_items.root {
            writeln!(f, "lang {}", marker.role)?;
            context.write_indent(f)?;
        }
        if self.exported {
            write!(f, "< ")?;
        }
        write!(f, "enum ")?;
        self.name.fmt_with(context, f)?;
        for (index, param) in self.generic_params.iter().enumerate() {
            write!(f, " ")?;
            if param.kind.is_some() && param.default.is_none() {
                write!(f, "(")?;
            }
            param.fmt_with(context, f)?;
            if param.kind.is_some() && param.default.is_none() {
                write!(f, ")")?;
            }
            if index + 1 < self.generic_params.len() {
                write!(f, ",")?;
            }
        }
        writeln!(f)?;

        context.increase_indent();
        let mut emitted_markers = vec![false; self.language_items.members.len()];
        for variant in &self.variants {
            write_language_item_member_marker(
                &self.language_items,
                LanguageItemMemberKind::Variant,
                &variant.name.name,
                &mut emitted_markers,
                context,
                f,
            )?;
            context.write_indent(f)?;
            variant.fmt_with(context, f)?;
            writeln!(f)?;
        }
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for EnumVariant {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.name.fmt_with(context, f)?;
        self.fields.fmt_with(context, f)
    }
}

impl FormatNode for NamedFieldsOrTypesList {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            NamedFieldsOrTypesList::NamedFields(fields) => {
                if fields.is_empty() {
                    return Ok(());
                }

                writeln!(f)?;
                context.increase_indent();
                for field in fields {
                    context.write_indent(f)?;
                    field.fmt_with(context, f)?;
                    writeln!(f)?;
                }
                context.decrease_indent();

                Ok(())
            }
            NamedFieldsOrTypesList::TypesList(types) => {
                for (i, ty) in types.iter().enumerate() {
                    write!(f, " ")?;
                    ty.fmt_with(context, f)?;

                    if i < types.len() - 1 {
                        write!(f, ",")?;
                    }
                }

                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;

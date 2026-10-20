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
                    if param.kind.is_some() && param.default.is_none() { write!(f, "(")?; }
                    param.fmt_with(context, f)?;
                    if param.kind.is_some() && param.default.is_none() { write!(f, ")")?; }
                    if index + 1 < params.len() { write!(f, ",")?; }
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
            if param.kind.is_some() && param.default.is_none() { write!(f, "(")?; }
            param.fmt_with(context, f)?;
            if param.kind.is_some() && param.default.is_none() { write!(f, ")")?; }
            if index + 1 < self.generic_params.len() { write!(f, ",")?; }
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
mod tests {
    use crate::{lexer::Span, parser};

    use super::*;

    #[test]
    fn formatter_does_not_expose_ast_display_compatibility() {
        for source in [
            include_str!("mod.rs"),
            include_str!("decl.rs"),
            include_str!("expr.rs"),
            include_str!("macros.rs"),
            include_str!("pattern.rs"),
        ] {
            assert!(!source.contains(concat!("impl", "_display_with_context")));
            assert!(!source.contains(concat!("impl std::fmt::", "Display")));
        }
    }

    fn ident(name: &str) -> Ident {
        Ident { context: Default::default(),
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn type_inner(name: &str) -> ParseTypeInner {
        ParseTypeInner { context: Default::default(),
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        }
    }

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(type_inner(name))
    }

    fn number_expr(value: u64) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn module(is_inline: bool, top_levels: Vec<TopLevel>) -> Module {
        Module {
            name: None,
            top_levels,
            is_inline,
            filepath: None,
        }
    }

    fn assert_source_fixture_formats(source: &str, expected: &str) {
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("fixture source should parse before formatting");
        let formatted = format(FormatInput::module_with_source(&program.module, source));
        assert_eq!(formatted, expected);

        let reparsed = parser::parse_string(&formatted, &crate::Config::default())
            .expect("formatted fixture should remain parseable");
        assert_eq!(
            format(FormatInput::module_with_source(
                &reparsed.module,
                &formatted
            )),
            expected
        );
    }

    #[test]
    fn format_input_formats_module_like_convenience_function() {
        let formatted_module = module(
            true,
            vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Box"),
                generic_params: vec![],
                fields: vec![StructDeclField {
                    name: ident("value"),
                    ty: ParseType::Unit(crate::lexer::Span::test()),
                    public: false,
                    default: None,
                }],
                exported: false,
            })],
        );

        assert_eq!(
            format(FormatInput::module(&formatted_module)),
            "struct Box\n    value : ()\n"
        );
    }

    #[test]
    fn format_repeat_array_literal() {
        assert_source_fixture_formats(
            "main  =  ->\n    values:[U8;4] = [1 + 2;4]\n    0\n",
            "main = ->\n    values: [U8; 4] = [1 + 2; 4]\n    0\n",
        );
    }

    #[test]
    fn format_call_argument_holes() {
        assert_source_fixture_formats(
            "main  =  ->\n    section = combine 1,_,3\n    0\n",
            "main = ->\n    section = combine 1, _, 3\n    0\n",
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_standalone_trivia() {
        let formatted_module = module(
            true,
            vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Box"),
                generic_params: vec![],
                fields: vec![StructDeclField {
                    name: ident("value"),
                    ty: ParseType::Unit(crate::lexer::Span::test()),
                    public: false,
                    default: None,
                }],
                exported: false,
            })],
        );
        let source = "// module docs\n\n/* block opener\n * block body\n */\nstruct   Box\n    value   :   ()\n";

        assert_eq!(
            format(FormatInput::module_with_source(&formatted_module, source)),
            "// module docs\n\n/* block opener\n * block body\n */\nstruct Box\n    value : ()\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_trivia_from_collapsed_body_lines() {
        let source = "main = ->\n    // body comment\n\n    0\n";
        let formatted_module = module(
            true,
            vec![TopLevel::FunctionDecl(FunctionDecl {
                name: ident("main"),
                lambda: LambdaDecl {
                    parameters: vec![],
                    body: Block {
                        statements: vec![Statement::Expression(number_expr(0))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            })],
        );

        assert_eq!(
            format(FormatInput::module_with_source(&formatted_module, source)),
            "main = -> 0\n    // body comment\n\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_standalone_multiline_block_comments() {
        let source = "/*\nmodule docs\n*/\nmain  =  ->  0\n";
        let formatted_module = module(
            true,
            vec![TopLevel::FunctionDecl(FunctionDecl {
                name: ident("main"),
                lambda: LambdaDecl {
                    parameters: vec![],
                    body: Block {
                        statements: vec![Statement::Expression(number_expr(0))],
                    },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            })],
        );

        assert_eq!(
            format(FormatInput::module_with_source(&formatted_module, source)),
            "/*\nmodule docs\n*/\nmain = -> 0\n"
        );
    }

    #[test]
    fn format_input_module_with_source_anchors_trivia_to_next_top_level() {
        let source = "foo = ->\n    0\n\n// bar docs\nbar = -> 1\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "foo = -> 0\n\n// bar docs\nbar = -> 1\n"
        );
    }

    #[test]
    fn format_input_module_with_source_keeps_operator_functions_as_code() {
        let source = "*  =  ->  1\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("operator function should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "* = -> 1\n"
        );
    }

    #[test]
    fn format_function_shorthand_uses_desugared_lambda_shape() {
        let source = "plus_one = (+ 1)\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function shorthand should parse before formatting");

        assert_eq!(
            format(FormatInput::program(&program)),
            "plus_one = x -> x + 1\n"
        );
    }

    #[test]
    fn format_unit_returning_lambda_preserves_arrow() {
        assert_source_fixture_formats("discard = value!->value\n", "discard = value !-> value\n");
    }

    #[test]
    fn format_native_range_forms() {
        assert_source_fixture_formats(
            "ranges = ->\n    a=1..4\n    b=1..=4\n    c=..4\n    d=..=4\n    e=1..\n    f=..\n",
            "ranges = ->\n    a = 1..4\n    b = 1..=4\n    c = ..4\n    d = ..=4\n    e = 1..\n    f = ..\n",
        );
    }

    #[test]
    fn format_hkt_surface_syntax_roundtrips() {
        let source = "trait Functor for F _\n\nstruct Compose (F _), (G _), A\n\ntype ResultWith E = \\T -> Result T, E\n\nimpl Functor for Result _, E\n\napply_f: F A -> A where F _: Functor\nconstructor_identity: F A -> F A where F _\nidentity: T -> T\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("HKT syntax should parse before formatting");
        let formatted = format(FormatInput::program(&program));
        let reparsed = parser::parse_string(&formatted, &crate::Config::default())
            .expect("formatted HKT syntax should parse");
        assert_eq!(format(FormatInput::program(&reparsed)), formatted);
    }

    #[test]
    fn format_input_module_with_source_preserves_struct_field_docs_before_field() {
        let source = "struct Box\n    // field docs\n    value : ()\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("struct source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "struct Box\n    // field docs\n    value : ()\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_statement_docs_between_statements() {
        let source = "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = ->\n    a = 1\n    // keep between statements\n    b = 2\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_block_then_line_statement_docs() {
        let source = "main = ->\n    /* docs */ // extra\n    0\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("block comment plus line comment should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = -> 0\n    /* docs */ // extra\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_top_level_trailing_comment() {
        let source = "main  =  ->  0  // keep main\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = -> 0 // keep main\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_statement_trailing_comment() {
        let source = "main = ->\n    value  =  1  // keep value\n    value\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = ->\n    value = 1 // keep value\n    value\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_collapsed_statement_trailing_comment() {
        let source = "main = ->\n    0  // keep return\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = -> 0 // keep return\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_multiple_collapsed_trailing_comments() {
        let source = "main = -> // keep header\n    0  // keep return\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("function source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "main = -> 0 // keep header\n    // keep return\n"
        );
    }

    #[test]
    fn format_preserves_constructor_valued_associated_type_binders() {
        let source = "trait Families\n    type Family _, _\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("constructor-valued associated type should parse before formatting");

        assert_eq!(format(FormatInput::module(&program.module)), source);
    }

    #[test]
    fn format_input_module_with_source_preserves_same_line_block_comment_before_code() {
        let source = "/* doc */ main  =  ->  0\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("same-line block comment source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "/* doc */\nmain = -> 0\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_block_comment_line_comment_remainder() {
        let source = "/* docs */ // extra\nmain  =  ->  0\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("block comment with line-comment remainder should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "/* docs */ // extra\nmain = -> 0\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_multiline_block_comment_line_comment_remainder() {
        let source = "/*\ndocs\n*/ // extra\nmain  =  ->  0\n";
        let program = parser::parse_string(source, &crate::Config::default()).expect(
            "multiline block comment with line-comment remainder should parse before formatting",
        );

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "/*\ndocs\n*/ // extra\nmain = -> 0\n"
        );
    }

    #[test]
    fn format_input_module_with_source_preserves_multiline_block_comment_before_code() {
        let source = "/*\nmodule docs\n*/ main  =  ->  0\n";
        let program = parser::parse_string(source, &crate::Config::default())
            .expect("closing-line block comment source should parse before formatting");

        assert_eq!(
            format(FormatInput::module_with_source(&program.module, source)),
            "/*\nmodule docs\n*/\nmain = -> 0\n"
        );
    }

    #[test]
    fn format_input_module_with_source_fixture_covers_declarations_and_module_trailing_trivia() {
        assert_source_fixture_formats(
            "// module docs\n\nstruct  Box\n    // field docs\n    value  :  I64  // field tail\n\nmain  =  ->  0  // main tail\n\n// module tail\n",
            "// module docs\n\nstruct Box\n    // field docs\n    value : I64 // field tail\n\nmain = -> 0 // main tail\n\n// module tail\n",
        );
    }

    #[test]
    fn format_input_module_with_source_fixture_covers_nested_blocks_and_patterns() {
        assert_source_fixture_formats(
            "main  =  ->\n    value  =  1\n    // before match\n    match value\n        0  if  value  ==  0  =>  0  // zero arm\n        _  =>\n            // nested body\n            value  // wildcard arm\n",
            "main = ->\n    value = 1\n    // before match\n    match value\n        0 if value == 0 => 0 // zero arm\n        _ => value // wildcard arm\n            // nested body\n",
        );
    }

    #[test]
    fn format_input_module_with_source_fixture_covers_macro_syntax_trivia() {
        assert_source_fixture_formats(
            "// macro docs\nmacro  passthrough\n    $value:expr  =>\n        $value\n\n// invocation docs\n%passthrough 1  // invocation tail\n",
            "// macro docs\nmacro passthrough\n    $value:expr =>\n        $value\n\n// invocation docs\n%passthrough 1 // invocation tail\n",
        );
    }

    #[test]
    fn format_language_item_protocols_preserves_all_markers() {
        let source = "lang sized\n< trait Sized\n\nlang drop\n< trait Drop\n    lang method\n    drop : I64\n\nlang index\n< trait Index\n    lang output\n    type Output\n    lang method\n    index : I64\n\nlang try\n< trait Try\n    lang output\n    type Output\n    lang residual\n    type Residual\n    lang branch\n    branch : I64\n\nlang from_residual\n< trait FromResidual\n    lang method\n    from_residual : I64\n\nlang control_flow\n< enum ControlFlow\n    lang break\n    Break\n    lang continue\n    Continue\n";
        let expected = source;

        assert_source_fixture_formats(source, expected);
    }

    #[test]
    fn format_language_item_method_marker_is_emitted_once_for_same_named_signature_and_method() {
        for source in [
            "trait Carrier\n    lang method\n    split : I64\n    split = -> 0\n",
            "trait Carrier\n    split : I64\n    lang method\n    split = -> 0\n",
        ] {
            let program = parser::parse_string(source, &crate::Config::default())
                .expect("same-named trait signature and default method should parse");
            let formatted = format(FormatInput::module(&program.module));

            assert_eq!(
                formatted
                    .lines()
                    .filter(|line| *line == "    lang method")
                    .count(),
                1,
                "formatted source:\n{formatted}",
            );

            let reparsed = parser::parse_string(&formatted, &crate::Config::default())
                .expect("formatted source should parse");
            let TopLevel::TraitDecl(trait_decl) = &reparsed.module.top_levels[0] else {
                panic!("expected a trait declaration");
            };
            assert_eq!(trait_decl.language_items.members.len(), 1);
            assert_eq!(
                trait_decl.language_items.members[0].kind,
                LanguageItemMemberKind::Method,
            );
            assert_eq!(trait_decl.language_items.members[0].member_name, "split");
        }
    }

    #[test]
    fn format_input_formats_program_like_convenience_function() {
        let program = Program {
            module: module(
                true,
                vec![TopLevel::FunctionSig(FunctionSig {
                    name: ident("main"),
                    sig: ParseType::Function(vec![named_type("I64")]),
                    where_clauses: vec![],
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                })],
            ),
        };

        assert_eq!(format(FormatInput::program(&program)), "main : I64\n");
    }

    #[test]
    fn format_context_formats_modules_without_global_state() {
        let indented_module = module(
            false,
            vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Box"),
                generic_params: vec![],
                fields: vec![StructDeclField {
                    name: ident("value"),
                    ty: ParseType::Unit(crate::lexer::Span::test()),
                    public: false,
                    default: None,
                }],
                exported: false,
            })],
        );
        let function_type_module = module(
            true,
            vec![TopLevel::FunctionSig(FunctionSig {
                name: ident("map"),
                sig: ParseType::Function(vec![
                    named_type("I64"),
                    ParseType::Function(vec![named_type("Bool"), named_type("I64")]),
                    ParseType::Unit(crate::lexer::Span::test()),
                ]),
                where_clauses: vec![],
                self_receiver: None,
                is_unsafe: false,
                exported: false,
            })],
        );

        let mut reused_context = FormatContext::new();
        assert_eq!(
            reused_context.format(FormatInput::module(&indented_module)),
            "    struct Box\n        value : ()\n"
        );
        assert_eq!(
            reused_context.format(FormatInput::module(&function_type_module)),
            "map : I64 -> (Bool -> I64) -> ()\n"
        );

        let mut independent_context = FormatContext::new();
        assert_eq!(
            independent_context.format(FormatInput::module(&function_type_module)),
            "map : I64 -> (Bool -> I64) -> ()\n"
        );
    }
}

use std::fmt::{self, Write};

use crate::ast::*;

use super::{write_language_item_member_marker, FormatContext, FormatNode};

impl FormatNode for TraitDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(marker) = &self.language_items.root {
            writeln!(f, "lang {}", marker.role)?;
            context.write_indent(f)?;
        }
        if self.exported {
            write!(f, "< ")?;
        }
        write!(f, "trait ")?;
        self.name.fmt_with(context, f)?;
        for (index, param) in self.generic_params.iter().enumerate() {
            write!(f, " ")?;
            let parenthesized = param.kind.as_ref().is_some_and(|kind| kind.args.is_empty());
            if parenthesized {
                write!(f, "(")?;
            }
            param.fmt_with(context, f)?;
            if parenthesized {
                write!(f, ")")?;
            }
            if index + 1 < self.generic_params.len() {
                write!(f, ",")?;
            }
        }
        if let Some(for_) = &self.for_ {
            write!(f, " for ")?;
            let parenthesized = for_.kind.as_ref().is_some_and(|kind| kind.args.is_empty());
            if parenthesized {
                write!(f, "(")?;
            }
            for_.fmt_with(context, f)?;
            if parenthesized {
                write!(f, ")")?;
            }
        }
        if !self.where_clauses.is_empty() {
            write!(f, " where ")?;
            for (index, clause) in self.where_clauses.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                clause.fmt_with(context, f)?;
            }
        }
        writeln!(f)?;

        context.increase_indent();
        let mut emitted_markers = vec![false; self.language_items.members.len()];
        for associated_type in &self.associated_types {
            write_language_item_member_marker(
                &self.language_items,
                LanguageItemMemberKind::AssociatedType,
                &associated_type.name.name,
                &mut emitted_markers,
                context,
                f,
            )?;
            context.write_indent(f)?;
            write!(f, "type ")?;
            let parenthesized = associated_type
                .kind
                .as_ref()
                .is_some_and(|kind| kind.args.is_empty());
            if parenthesized {
                write!(f, "(")?;
            }
            associated_type.name.fmt_with(context, f)?;
            if let Some(kind) = &associated_type.kind {
                if kind.args.is_empty() {
                    write!(f, ": ")?;
                    kind.constructor.fmt_with(context, f)?;
                } else {
                    for index in 0..kind.args.len() {
                        if index == 0 {
                            write!(f, " _")?;
                        } else {
                            write!(f, ", _")?;
                        }
                    }
                }
            }
            if parenthesized {
                write!(f, ")")?;
            }
            writeln!(f)?;
        }
        for (_name, signature) in &self.signatures {
            write_language_item_member_marker(
                &self.language_items,
                LanguageItemMemberKind::Method,
                &signature.name.name,
                &mut emitted_markers,
                context,
                f,
            )?;
            context.write_indent(f)?;
            signature.fmt_with(context, f)?;
        }
        for (_, method) in &self.methods {
            write_language_item_member_marker(
                &self.language_items,
                LanguageItemMemberKind::Method,
                &method.name.name,
                &mut emitted_markers,
                context,
                f,
            )?;
            context.write_indent(f)?;
            method.fmt_with(context, f)?;
        }
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for Impl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "impl ")?;
        self.name.fmt_with(context, f)?;
        if let Some(for_) = &self.for_ {
            write!(f, " for ")?;
            for_.fmt_with(context, f)?;
        }
        if !self.where_clauses.is_empty() {
            write!(f, " where ")?;
            for (index, clause) in self.where_clauses.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                clause.fmt_with(context, f)?;
            }
        }
        writeln!(f)?;

        context.increase_indent();
        for associated_type in &self.associated_types {
            context.write_indent(f)?;
            write!(f, "type ")?;
            let parenthesized = associated_type
                .kind
                .as_ref()
                .is_some_and(|kind| kind.args.is_empty());
            if parenthesized {
                write!(f, "(")?;
            }
            associated_type.name.fmt_with(context, f)?;
            if let Some(kind) = &associated_type.kind {
                if kind.args.is_empty() {
                    write!(f, ": ")?;
                    kind.constructor.fmt_with(context, f)?;
                } else {
                    for index in 0..kind.args.len() {
                        if index == 0 {
                            write!(f, " _")?;
                        } else {
                            write!(f, ", _")?;
                        }
                    }
                }
            }
            if parenthesized {
                write!(f, ")")?;
            }
            write!(f, " = ")?;
            associated_type.ty.fmt_with(context, f)?;
            writeln!(f)?;
        }
        for (_name, signature) in &self.signatures {
            context.write_indent(f)?;
            signature.fmt_with(context, f)?;
        }
        for (_, method) in &self.methods {
            context.write_indent(f)?;
            method.fmt_with(context, f)?;
        }
        context.decrease_indent();

        Ok(())
    }
}

impl FormatNode for Path {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            Path::Ident(ident) => ident.fmt_with(context, f),
            Path::Type(ty) => ty.fmt_with(context, f),
        }
    }
}

impl FormatNode for IdentifierPath {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        for (i, ident) in self.path.iter().enumerate() {
            format_path_segment(ident, context, f)?;

            if i < self.path.len() - 1 {
                write!(f, "::")?;
            }
        }

        Ok(())
    }
}

impl FormatNode for TypePath {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        for (i, ident) in self.path.iter().enumerate() {
            format_path_segment(ident, context, f)?;

            if i < self.path.len() - 1 {
                write!(f, "::")?;
            }
        }

        Ok(())
    }
}

fn format_path_segment<W: Write>(
    segment: &IdentOrType,
    context: &mut FormatContext,
    f: &mut W,
) -> fmt::Result {
    let needs_parentheses = matches!(
        segment,
        IdentOrType::Type(
            ParseType::Application(_)
                | ParseType::Lambda(_)
                | ParseType::Function(_)
                | ParseType::Hole(_)
        )
    );
    if needs_parentheses {
        write!(f, "(")?;
    }
    segment.fmt_with(context, f)?;
    if needs_parentheses {
        write!(f, ")")?;
    }
    Ok(())
}

impl FormatNode for IdentOrType {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            IdentOrType::Ident(ident) => ident.fmt_with(context, f),
            IdentOrType::Type(parse_type) => parse_type.fmt_with(context, f),
        }
    }
}

impl FormatNode for Ident {
    fn fmt_with<W: Write>(&self, _context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl FormatNode for ParseType {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        match self {
            ParseType::Object(object) => {
                object.base.fmt_with(context, f)?;
                write!(f, " {{ ")?;
                for (index, qualifier) in object.qualifiers.iter().enumerate() {
                    if index != 0 {
                        write!(f, ", ")?;
                    }
                    match qualifier {
                        crate::ast::ObjectQualifier::Trait(ty) => {
                            write!(f, "(")?;
                            ty.fmt_with(context, f)?;
                            write!(f, ")")?;
                        }
                        crate::ast::ObjectQualifier::Binding { owner, member, ty } => {
                            if let Some(owner) = owner {
                                write!(f, "(")?;
                                owner.fmt_with(context, f)?;
                                write!(f, ")::")?;
                            }
                            member.fmt_with(context, f)?;
                            write!(f, " = (")?;
                            ty.fmt_with(context, f)?;
                            write!(f, ")")?;
                        }
                    }
                }
                write!(f, " }}")
            }
            ParseType::Function(types) => {
                let was_inside_fn_type_decl = context.inside_fn_type_decl;
                if was_inside_fn_type_decl {
                    write!(f, "(")?;
                } else {
                    context.inside_fn_type_decl = true;
                }

                for (i, inner) in types.iter().enumerate() {
                    let needs_parentheses = matches!(inner, ParseType::Lambda(_));
                    if needs_parentheses {
                        write!(f, "(")?;
                    }
                    inner.fmt_with(context, f)?;
                    if needs_parentheses {
                        write!(f, ")")?;
                    }

                    if i < types.len() - 1 {
                        write!(f, " -> ")?;
                    }
                }

                context.inside_fn_type_decl = was_inside_fn_type_decl;

                if was_inside_fn_type_decl {
                    write!(f, ")")?;
                }

                Ok(())
            }
            ParseType::Slice(inner) => {
                write!(f, "[")?;
                inner.fmt_with(context, f)?;
                write!(f, "]")
            }
            ParseType::Array { inner, len } => {
                write!(f, "[")?;
                inner.fmt_with(context, f)?;
                write!(f, "; {}]", len)
            }
            ParseType::Tuple(types) => {
                write!(f, "(")?;

                for (i, inner) in types.iter().enumerate() {
                    inner.fmt_with(context, f)?;

                    if i < types.len() - 1 {
                        write!(f, ", ")?;
                    }
                }

                write!(f, ")")
            }
            ParseType::Type(inner) => inner.fmt_with(context, f),
            ParseType::Application(application) => application.fmt_with(context, f),
            ParseType::Lambda(lambda) => lambda.fmt_with(context, f),
            ParseType::Hole(hole) => hole.fmt_with(context, f),
            ParseType::Associated { base, member } => {
                base.fmt_with(context, f)?;
                write!(f, "::")?;
                member.fmt_with(context, f)
            }
            ParseType::Reference { is_mut, pointee } => {
                if *is_mut {
                    write!(f, "&mut ")?;
                } else {
                    write!(f, "&")?;
                }

                pointee.fmt_with(context, f)
            }
            ParseType::Pointer(pointee) => {
                write!(f, "*")?;
                pointee.fmt_with(context, f)
            }
            ParseType::Unit(_) => write!(f, "()"),
        }
    }
}

impl FormatNode for ParseTypeInner {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "{}", self.name)?;

        for (i, generic) in self.generics.iter().enumerate() {
            write!(f, " ")?;
            generic.fmt_with(context, f)?;

            if i < self.generics.len() - 1 {
                write!(f, ",")?;
            }
        }

        Ok(())
    }
}

impl FormatNode for GenericParamDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        if let Some(bound) = &self.unsized_bound {
            write!(f, "(")?;
            self.name.fmt_with(context, f)?;
            write!(f, ": ?")?;
            bound.fmt_with(context, f)?;
            return write!(f, ")");
        }
        self.name.fmt_with(context, f)?;
        if let Some(kind) = &self.kind {
            if kind.args.is_empty() {
                write!(f, ": ")?;
                kind.constructor.fmt_with(context, f)?;
            } else {
                for (index, arg) in kind.args.iter().enumerate() {
                    write!(f, " ")?;
                    arg.fmt_with(context, f)?;
                    if index + 1 < kind.args.len() {
                        write!(f, ",")?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl FormatNode for TypeApplication {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        let needs_parentheses = matches!(self.constructor.as_ref(), ParseType::Lambda(_))
            || (matches!(self.constructor.as_ref(), ParseType::Function(_))
                && !context.inside_fn_type_decl);
        if needs_parentheses {
            write!(f, "(")?;
        }
        self.constructor.fmt_with(context, f)?;
        if needs_parentheses {
            write!(f, ")")?;
        }
        for (index, arg) in self.args.iter().enumerate() {
            write!(f, " ")?;
            let arg_needs_parentheses =
                matches!(arg, ParseType::Application(_) | ParseType::Lambda(_))
                    || (matches!(arg, ParseType::Function(_)) && !context.inside_fn_type_decl);
            if arg_needs_parentheses {
                write!(f, "(")?;
            }
            arg.fmt_with(context, f)?;
            if arg_needs_parentheses {
                write!(f, ")")?;
            }
            if index + 1 < self.args.len() {
                write!(f, ",")?;
            }
        }
        Ok(())
    }
}

impl FormatNode for TypeLambda {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "\\")?;
        for (index, param) in self.params.iter().enumerate() {
            if index > 0 {
                write!(f, ", ")?;
            }
            if param.kind.is_some() {
                write!(f, "(")?;
                param.fmt_with(context, f)?;
                write!(f, ")")?;
            } else {
                param.fmt_with(context, f)?;
            }
        }
        write!(f, " -> ")?;
        self.body.fmt_with(context, f)
    }
}

impl FormatNode for TypeHole {
    fn fmt_with<W: Write>(&self, _context: &mut FormatContext, f: &mut W) -> fmt::Result {
        write!(f, "_")
    }
}

impl FormatNode for WhereClause {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        self.subject.fmt_with(context, f)?;
        if let Some(trait_bound) = &self.trait_bound {
            write!(f, ": ")?;
            if self.relaxed {
                write!(f, "?")?;
            }
            trait_bound.fmt_with(context, f)?;
        }
        Ok(())
    }
}

impl FormatNode for FunctionSig {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        let unsafe_prefix = if self.is_unsafe { "unsafe " } else { "" };
        let self_receiver = self
            .self_receiver
            .map(|mode| match mode {
                SelfReceiverMode::Shared => "@",
                SelfReceiverMode::Mut => "^@",
                SelfReceiverMode::Move => "~@",
            })
            .unwrap_or("");
        write!(f, "{}{}", unsafe_prefix, self_receiver)?;
        self.name.fmt_with(context, f)?;
        write!(f, " : ")?;
        self.sig.fmt_with(context, f)?;
        if !self.where_clauses.is_empty() {
            write!(f, " where ")?;
            for (index, clause) in self.where_clauses.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                clause.subject.fmt_with(context, f)?;
                if let Some(trait_bound) = &clause.trait_bound {
                    write!(f, ": ")?;
                    if clause.relaxed {
                        write!(f, "?")?;
                    }
                    trait_bound.fmt_with(context, f)?;
                }
            }
        }
        writeln!(f)
    }
}

impl FormatNode for FunctionDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        let unsafe_prefix = if self.is_unsafe { "unsafe " } else { "" };
        let self_receiver = self
            .self_receiver
            .map(|mode| match mode {
                SelfReceiverMode::Shared => "@",
                SelfReceiverMode::Mut => "^@",
                SelfReceiverMode::Move => "~@",
            })
            .unwrap_or("");
        write!(f, "{}{}", unsafe_prefix, self_receiver)?;
        self.name.fmt_with(context, f)?;
        write!(f, " = ")?;
        self.lambda.fmt_with(context, f)?;
        writeln!(f)
    }
}

impl FormatNode for LambdaDecl {
    fn fmt_with<W: Write>(&self, context: &mut FormatContext, f: &mut W) -> fmt::Result {
        for (i, param) in self.parameters.iter().enumerate() {
            param.fmt_with(context, f)?;

            if i < self.parameters.len() - 1 {
                write!(f, ", ")?;
            }
        }

        if !self.parameters.is_empty() {
            write!(f, " ")?;
        }

        let arrow = match self.arrow_kind {
            LambdaArrowKind::Normal => "->",
            LambdaArrowKind::Unit => "!->",
            LambdaArrowKind::Curried => "~>",
        };

        write!(f, "{}", arrow)?;

        if self.body.statements.len() <= 1 {
            write!(f, " ")?;
        }

        display_block(context, &self.body, false, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Span;

    fn format_node<T: FormatNode>(node: &T) -> String {
        let mut context = FormatContext::new();
        let mut output = String::new();
        node.fmt_with(&mut context, &mut output)
            .expect("formatting into a String should not fail");
        output
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    #[test]
    fn function_sig_formats_self_receiver_prefixes() {
        let cases = [
            (Some(SelfReceiverMode::Shared), "@a : ()\n"),
            (Some(SelfReceiverMode::Mut), "^@a : ()\n"),
            (Some(SelfReceiverMode::Move), "~@a : ()\n"),
            (None, "a : ()\n"),
        ];

        for (self_receiver, expected) in cases {
            let sig = FunctionSig {
                name: ident("a"),
                sig: ParseType::Unit(crate::lexer::Span::test()),
                where_clauses: vec![],
                self_receiver,
                is_unsafe: false,
                exported: false,
            };

            assert_eq!(format_node(&sig), expected);
        }
    }

    #[test]
    fn function_decl_formats_self_receiver_prefixes() {
        let cases = [
            (Some(SelfReceiverMode::Shared), "@a = -> \n"),
            (Some(SelfReceiverMode::Mut), "^@a = -> \n"),
            (Some(SelfReceiverMode::Move), "~@a = -> \n"),
            (None, "a = -> \n"),
        ];

        for (self_receiver, expected) in cases {
            let decl = FunctionDecl {
                name: ident("a"),
                lambda: LambdaDecl {
                    parameters: vec![],
                    body: Block { statements: vec![] },
                    arrow_kind: LambdaArrowKind::Normal,
                    span: crate::lexer::Span::test(),
                },
                self_receiver,
                is_unsafe: false,
                exported: false,
            };

            assert_eq!(format_node(&decl), expected);
        }
    }
}

pub(super) fn display_block<W: Write>(
    context: &mut FormatContext,
    block: &Block,
    force_multiline: bool,
    f: &mut W,
) -> fmt::Result {
    let mono_statement = !force_multiline && block.statements.len() <= 1;

    if !mono_statement {
        context.increase_indent();
        writeln!(f)?;
    }

    for (i, stmt) in block.statements.iter().enumerate() {
        if !mono_statement {
            context.write_indent(f)?;
        }

        stmt.fmt_with(context, f)?;

        if !mono_statement && i < block.statements.len() - 1 {
            writeln!(f)?;
        }
    }

    if !mono_statement {
        context.decrease_indent();
    }

    Ok(())
}

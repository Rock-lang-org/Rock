use super::{visit::*, *};
use crate::fmt::{FormatContext, FormatNode};
use crate::walk_list;
use paste::paste;

pub fn debug_ast(ast: &Program) {
    ast.visit(&mut AstPrinter::default());
}

#[derive(Default)]
struct AstPrinter {
    indent_level: usize,
    #[cfg(test)]
    visited_idents: Vec<String>,
}

impl AstPrinter {
    fn indent(&self) -> String {
        "  ".repeat(self.indent_level)
    }

    fn name_with_indent<F>(&mut self, name: &str, f: F)
    where
        F: FnOnce(&mut Self),
    {
        self.name(name);

        self.indent_level += 1;

        f(self);

        self.indent_level -= 1;
    }

    fn name(&mut self, name: &str) {
        println!("{}{}", self.indent(), name);
    }

    fn name_value<T>(&mut self, name: &str, value: T)
    where
        T: std::fmt::Display,
    {
        println!("{}{} = {}", self.indent(), name, value);
    }

    fn name_formatted<T>(&mut self, name: &str, value: &T)
    where
        T: FormatNode,
    {
        self.name_value(name, format_node(value));
    }
}

fn format_node<T>(node: &T) -> String
where
    T: FormatNode,
{
    let mut context = FormatContext::new();
    let mut output = String::new();
    node.fmt_with(&mut context, &mut output)
        .expect("formatting into a String should not fail");
    output
}

fn print_language_item_annotations(
    printer: &mut AstPrinter,
    annotations: &LanguageItemAnnotations,
) {
    if let Some(marker) = &annotations.root {
        printer.name_value("LanguageItemRoot", marker.role);
    }

    for member in &annotations.members {
        printer.name_value(
            "LanguageItemMember",
            format!(
                "role={}, kind={:?}, name={}",
                member.marker.role, member.kind, member.member_name
            ),
        );
    }
}

macro_rules! ast_printer {
    ($(
        $name:ty
    )+) => {
        impl<'a> Visitor<'a> for AstPrinter {
            fn visit_name(&mut self, _name: &str) {}

            fn visit_primitive<T>(&mut self, _val: T)
            where
                T: std::fmt::Debug,
            {}

            fn visit_ident(&mut self, ident: &'a Ident) {
                #[cfg(test)]
                self.visited_idents.push(ident.name.clone());
                self.name_value(ident.name(), &ident.name);
            }

            fn visit_parse_type(&mut self, parse_type: &'a ParseType) {
                self.name_formatted(parse_type.name(), parse_type);
            }

            fn visit_parse_type_inner(&mut self, parse_type: &'a ParseTypeInner) {
                self.name_formatted(parse_type.name(), parse_type);
            }

            fn visit_operator(&mut self, operator: &'a Operator) {
                self.name_value(operator.name(), &operator.value);
            }

            fn visit_literal(&mut self, literal: &'a Literal) {
                match &literal.kind {
                    LiteralKind::Number(n) => {
                        self.name_value("Number", n);
                    }
                    LiteralKind::String(s) => {
                        self.name_value("String", s);
                    }
                    LiteralKind::Char(c) => {
                        self.name_value("Char", c);
                    }
                    LiteralKind::Float(f) => {
                        self.name_value("Float", f);
                    }
                    LiteralKind::Bool(b) => {
                        self.name_value("Bool", b);
                    }
                    LiteralKind::Array(array) => {
                        self.name_with_indent("Array", |printer| {
                            walk_list!(printer, visit_expression, &array.elements);
                        });
                    }
                    LiteralKind::ArrayRepeat { value, len } => {
                        self.name_with_indent("ArrayRepeat", |printer| {
                            printer.visit_expression(value);
                            printer.name_value("Length", len);
                        });
                    }
                }
            }

            fn visit_secondary_expr(&mut self, secondary_expr: &'a SecondaryExpr) {
                match secondary_expr {
                    SecondaryExpr::Dot(name) => {
                        println!("{}{} = .{}", self.indent(), "Dot", format_node(name));
                    }
                    SecondaryExpr::Arguments(args) => {
                        self.name_with_indent("Arguments", |printer| {
                            walk_list!(printer, visit_argument, args);
                        });
                    }
                    SecondaryExpr::Indice(expr) => {
                        self.name_with_indent("Indice", |printer| {
                            expr.visit(printer);
                        });
                    }
                    SecondaryExpr::Interogation => {
                        self.name("Interogation");
                    }
                }
            }

            fn visit_pattern(&mut self, pattern: &'a Pattern) {
                match &pattern.kind {
                    PatternKind::Instance(instance) => {
                        self.name_with_indent("InstancePattern", |printer| {
                            instance.visit(printer);
                        });
                    }
                    PatternKind::Ident(field) => {
                        self.name_formatted("IdentPattern", field);
                    }
                    PatternKind::Array(array) => {
                        self.name_with_indent("ArrayPattern", |printer| {
                            walk_list!(printer, visit_pattern, array);
                        });
                    }
                    PatternKind::Tuple(tuple) => {
                        self.name_with_indent("TuplePattern", |printer| {
                            walk_list!(printer, visit_pattern, tuple);
                        });
                    }
                    PatternKind::Literal(literal) => {
                        self.name_with_indent("LiteralPattern", |printer| {
                            literal.visit(printer);
                        });
                    }
                    PatternKind::Wildcard => {
                        self.name("WildcardPattern");
                    }
                    PatternKind::Nested(nested) => {
                        self.name_with_indent("NestedPattern", |printer| {
                            nested.visit(printer);
                        });
                    }
                    PatternKind::Reference { pattern, mutable } => {
                        self.name_with_indent("ReferencePattern", |printer| {
                            println!("{}mutable: {}", printer.indent(), mutable);
                            pattern.visit(printer);
                        });
                    }
                }
            }

            fn visit_trait_decl(&mut self, trait_decl: &'a TraitDecl) {
                self.name_with_indent("TraitDecl", |printer| {
                    print_language_item_annotations(printer, &trait_decl.language_items);
                    walk_trait_decl(printer, trait_decl);
                });
            }

            fn visit_enum_decl(&mut self, enum_decl: &'a EnumDecl) {
                self.name_with_indent("EnumDecl", |printer| {
                    print_language_item_annotations(printer, &enum_decl.language_items);
                    walk_enum_decl(printer, enum_decl);
                });
            }

            fn visit_assignment(&mut self, assignment: &'a Assignment) {
                self.name_with_indent("Assignment", |printer| {
                    printer.name_with_indent("LHS", |printer| {
                        assignment.lhs.visit(printer);
                    });

                    printer.name_with_indent("RHS", |printer| {
                        assignment.rhs.visit(printer);
                    });
                });

            }

            paste! {
                $(
                    fn [<visit_ $name:snake>](&mut self, node: &'a$name) {
                        self.name_with_indent(&node.name(), |printer| {
                            [<walk_ $name:snake>](printer, node);
                        });
                    }
                )+
            }
        }

    };
}

ast_printer!(
    Program
    ModuleDecl
    Module
    TopLevel
    MacroDecl
    MacroInvoc
    // TraitDecl
    Impl
    // EnumDecl
    EnumVariant
    // NamedFieldsOrTypesList
    FunctionDecl
    FunctionSig
    LambdaDecl
    // Block
    StructDecl
    StructDeclField
    // Ident
    // IdentOrNumber
    // Assignment
    // AssignmentLHS
    // IdentifierPath
    // Statement
    Loop
    // Expression
    If
    Else
    Match
    MatchArm
    // Pattern
    // PatternKind
    // InstancePattern
    // FieldPattern
    // ArrayPattern
    // FieldsPatternOrArgumentsPattern
    // UnaryExpr
    // Operator
    /* PrimaryExpr
    SecondaryExpr */
    // Operand
    Argument
    // Literal
    Instance
    NativeOperator
    Tuple
    Array
    IdentPattern
    // ParseType
    // ParseTypeInner
    // IdentOrType
);

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::lexer::Span;

    use super::*;

    fn ident(name: &str) -> Ident {
        Ident { context: Default::default(),
            name: name.to_string(),
            span: Span::test(),
        }
    }

    #[test]
    fn ast_printer_walks_associated_types_and_signatures_in_traits() {
        let signature_name = ident("split");
        let trait_decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner { context: Default::default(),
                name: "Carrier".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![],
            for_: None,
            associated_types: vec![AssociatedTypeDecl {
                name: ident("Value"),
                kind: None,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                signature_name.clone(),
                FunctionSig {
                    name: signature_name,
                    sig: ParseType::Unit(crate::lexer::Span::test()),
                    where_clauses: vec![],
                    self_receiver: None,
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            exported: false,
            language_items: Default::default(),
        };
        let mut printer = AstPrinter::default();

        trait_decl.visit(&mut printer);

        assert!(printer.visited_idents.iter().any(|name| name == "Value"));
        assert!(printer.visited_idents.iter().any(|name| name == "split"));
    }
}

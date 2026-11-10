use std::collections::HashMap;

use crate::ast;
use crate::lexer::Span;

use super::type_references::type_inner_name_span;

pub(super) fn top_level_declaration_span(top_level: &ast::TopLevel, name_span: &Span) -> Span {
    let mut end = name_span.end;
    let mut extend = |span: Span| {
        if span.file_path == name_span.file_path {
            end = end.max(span.end);
        }
    };
    match top_level {
        ast::TopLevel::FunctionDecl(function) => {
            for statement in &function.lambda.body.statements {
                if let Some(span) = ast_statement_span(statement) {
                    extend(span);
                }
            }
        }
        ast::TopLevel::FunctionSig(signature) | ast::TopLevel::Extern(signature) => {
            extend(signature.sig.span());
        }
        ast::TopLevel::StructDecl(structure) => {
            for generic in &structure.generic_params {
                extend(generic.span.clone());
            }
            for field in &structure.fields {
                extend(field.name.span.clone());
                extend(field.ty.span());
            }
        }
        ast::TopLevel::EnumDecl(enumeration) => {
            for variant in &enumeration.variants {
                extend(variant.name.span.clone());
                match &variant.fields {
                    ast::NamedFieldsOrTypesList::NamedFields(fields) => {
                        for field in fields {
                            extend(field.name.span.clone());
                            extend(field.ty.span());
                        }
                    }
                    ast::NamedFieldsOrTypesList::TypesList(types) => {
                        for ty in types {
                            extend(ty.span());
                        }
                    }
                }
            }
        }
        ast::TopLevel::TraitDecl(trait_decl) => {
            for generic in &trait_decl.generic_params {
                extend(generic.span.clone());
            }
            for clause in &trait_decl.where_clauses {
                extend(clause.subject.span());
                if let Some(bound) = &clause.trait_bound {
                    extend(bound.span());
                }
            }
            for method in trait_decl.methods.values() {
                extend(method.name.span.clone());
            }
            for signature in trait_decl.signatures.values() {
                extend(signature.sig.span());
            }
        }
        ast::TopLevel::Impl(implementation) => {
            for generic in implementation.name.generics() {
                extend(generic.span());
            }
            if let Some(receiver) = &implementation.for_ {
                extend(receiver.span());
            }
            for associated in &implementation.associated_types {
                extend(associated.name.span.clone());
                extend(associated.ty.span());
            }
            for method in implementation.methods.values() {
                extend(method.name.span.clone());
            }
            for signature in implementation.signatures.values() {
                extend(signature.sig.span());
            }
        }
        _ => {}
    }
    Span::new(name_span.file_path.clone(), name_span.start, end)
}

pub(super) fn top_level_name_span(top_level: &ast::TopLevel, indexed: &Span) -> Span {
    let (name, span) = match top_level {
        ast::TopLevel::StructDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::EnumDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::TraitDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::Impl(decl) => {
            return decl
                .name
                .named_inner()
                .map(type_inner_name_span)
                .unwrap_or_else(|| decl.name.span())
        }
        _ => return indexed.clone(),
    };
    Span::new(span.file_path.clone(), span.start, span.start + name.len())
}

fn ast_statement_span(statement: &ast::Statement) -> Option<Span> {
    match statement {
        ast::Statement::Assignment(assignment) => ast_expression_span(&assignment.rhs),
        ast::Statement::Expression(expression) => ast_expression_span(expression),
        ast::Statement::Return(expression)
        | ast::Statement::Continue(expression)
        | ast::Statement::Break(expression) => expression.as_ref().and_then(ast_expression_span),
    }
}

fn ast_expression_span(expression: &ast::Expression) -> Option<Span> {
    match expression {
        ast::Expression::BinopExpr(_, operator, _) => Some(operator.span.clone()),
        ast::Expression::UnaryExpr(unary) => match unary {
            ast::UnaryExpr::UnaryExpr(operator, _) => Some(operator.span.clone()),
            ast::UnaryExpr::PrimaryExpr(primary) => match &primary.operand {
                ast::Operand::Literal(literal) => Some(literal.span.clone()),
                ast::Operand::Ident(path) => path.path.first().map(|segment| match segment {
                    ast::IdentOrType::Ident(ident) => ident.span.clone(),
                    ast::IdentOrType::Type(ty) => ty.span(),
                }),
                ast::Operand::SelfIdent(ident) => Some(ident.span.clone()),
                ast::Operand::CallHole(span) => Some(span.clone()),
                ast::Operand::Instance(instance) => {
                    instance.name.path.first().map(|segment| match segment {
                        ast::IdentOrType::Ident(ident) => ident.span.clone(),
                        ast::IdentOrType::Type(ty) => ty.span(),
                    })
                }
                ast::Operand::Expression(expression) => ast_expression_span(expression),
                _ => None,
            },
        },
        ast::Expression::CastExpr(expression, ty) => {
            ast_expression_span(expression).or_else(|| Some(ty.span()))
        }
        ast::Expression::Range(range) => Some(range.span.clone()),
    }
}

pub(super) fn collect_parse_type_name_spans(
    ty: &ast::ParseType,
    output: &mut HashMap<String, Span>,
) {
    match ty {
        ast::ParseType::Type(inner) => {
            output
                .entry(inner.name.clone())
                .or_insert(inner.span.clone());
            for generic in &inner.generics {
                collect_parse_type_name_spans(generic, output);
            }
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                collect_parse_type_name_spans(ty, output);
            }
        }
        ast::ParseType::Application(application) => {
            collect_parse_type_name_spans(&application.constructor, output);
            for arg in &application.args {
                collect_parse_type_name_spans(arg, output);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            for param in &lambda.params {
                output
                    .entry(param.name.name.clone())
                    .or_insert(param.name.span.clone());
            }
            collect_parse_type_name_spans(&lambda.body, output);
        }
        ast::ParseType::Associated { base, member } => {
            output.entry(base.name.clone()).or_insert(base.span.clone());
            output
                .entry(member.name.clone())
                .or_insert(member.span.clone());
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => collect_parse_type_name_spans(inner, output),
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => {}
    }
}

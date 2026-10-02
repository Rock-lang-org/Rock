use paste::paste;

use crate::ast::tree::*;

#[macro_export]
macro_rules! walk_list {
    ($visitor:expr, $method:ident, $list:expr) => {
        for elem in $list {
            elem.visit($visitor);
        }
    };

    ($visitor:expr, $method:ident, $list:expr, $($extra_args:expr),*) => {
        for elem in $list {
            $visitor.$method(elem, $($extra_args,)*)
        }
    }
}

macro_rules! walk_map {
    ($visitor:expr, $list:expr) => {
        for (k, elem) in $list {
            k.visit($visitor);
            elem.visit($visitor);
        }
    };
}

macro_rules! generate_visitor_trait {
    ($(
        $name:ty
    )+) => {
        pub trait Visitor<'ast>: Sized {
            fn visit_name(&mut self, _name: &str) {}

            fn visit_primitive<T>(&mut self, _val: T)
            where
                T: std::fmt::Debug,
            {}

            paste! {
                $(
                    fn [<visit_ $name:snake>](&mut self, node: &'ast $name) {
                        [<walk_ $name:snake>](self, node);
                    }
                )+
            }
        }

        $(
            impl $name {
                pub fn visit<'ast, T: Visitor<'ast>>(&'ast self, visitor: &mut T) {
                    paste! {
                        visitor.[<visit_ $name:snake>](self);
                    }
                }

                pub fn name(&self) -> &'static str {
                    stringify!($name)
                }
            }
        )+
    };
}

generate_visitor_trait!(
    Program
    ModuleDecl
    Module
    TopLevel
    MacroDecl
    MacroInvoc
    TraitDecl
    Impl
    EnumDecl
    EnumVariant
    NamedFieldsOrTypesList
    FunctionDecl
    FunctionSig
    LambdaDecl
    Block
    StructDecl
    StructDeclField
    Ident
    IdentOrNumber
    IdentPattern
    Assignment
    AssignmentLHS
    Path
    IdentifierPath
    TypePath
    Statement
    Loop
    Expression
    Condition
    If
    Else
    Match
    MatchArm
    Pattern
    PatternKind
    InstancePattern
    FieldPattern
    ArrayPattern
    FieldsPatternOrArgumentsPattern
    UnaryExpr
    Operator
    PrimaryExpr
    SecondaryExpr
    Operand
    Argument
    Literal
    Instance
    NativeOperator
    Tuple
    Array
    ParseType
    ParseTypeInner
    GenericParamDecl
    TypeApplication
    TypeLambda
    TypeHole
    IdentOrType
    WhereClause
);

pub fn walk_root<'a, V: Visitor<'a>>(visitor: &mut V, root: &'a Program) {
    visitor.visit_module(&root.module);
}

pub fn walk_module<'a, V: Visitor<'a>>(visitor: &mut V, r#mod: &'a Module) {
    if let Some(name) = &r#mod.name {
        visitor.visit_ident(name);
    }

    walk_list!(visitor, visit_top_level, &r#mod.top_levels);
}

pub fn walk_module_decl<'a, V: Visitor<'a>>(visitor: &mut V, module_decl: &'a ModuleDecl) {
    visitor.visit_module(&module_decl.0);
}

pub fn walk_top_level<'a, V: Visitor<'a>>(visitor: &mut V, top_level: &'a TopLevel) {
    match &top_level {
        TopLevel::Module(m) => visitor.visit_module_decl(m),
        TopLevel::Mod(ident, _) => visitor.visit_ident(ident),
        TopLevel::Import(path) => visitor.visit_path(path),
        TopLevel::GlobImport(_) => (),
        TopLevel::Export(path) => visitor.visit_path(path),
        TopLevel::GlobExport(_) => (),
        TopLevel::InfixOperator(_precedence, _name) => (),
        TopLevel::MacroDecl(m) => visitor.visit_macro_decl(m),
        TopLevel::MacroInvoc(m) => visitor.visit_macro_invoc(m),
        TopLevel::Extern(sig) => visitor.visit_function_sig(sig),
        TopLevel::FunctionSig(sig) => visitor.visit_function_sig(sig),
        TopLevel::FunctionDecl(f) => visitor.visit_function_decl(f),
        TopLevel::StructDecl(i) => visitor.visit_struct_decl(i),
        TopLevel::TraitDecl(t) => visitor.visit_trait_decl(t),
        TopLevel::EnumDecl(e) => visitor.visit_enum_decl(e),
        TopLevel::Impl(i) => visitor.visit_impl(i),
        TopLevel::NewType(inner, ty) => {
            visitor.visit_parse_type_inner(inner);
            visitor.visit_parse_type(ty);
        }
    };
}

pub fn walk_struct_decl<'a, V: Visitor<'a>>(visitor: &mut V, s: &'a StructDecl) {
    visitor.visit_parse_type_inner(&s.name);

    walk_list!(visitor, visit_generic_param_decl, &s.generic_params);
    walk_list!(visitor, visit_struct_decl_field, &s.fields);
}

pub fn walk_struct_decl_field<'a, V: Visitor<'a>>(visitor: &mut V, s: &'a StructDeclField) {
    visitor.visit_ident(&s.name);
    visitor.visit_parse_type(&s.ty);
    if let Some(e) = &s.default {
        visitor.visit_expression(e);
    }
}

pub fn walk_trait<'a, V: Visitor<'a>>(visitor: &mut V, t: &'a TraitDecl) {
    visitor.visit_parse_type_inner(&t.name);

    walk_list!(visitor, visit_generic_param_decl, &t.generic_params);

    if let Some(for_) = &t.for_ {
        visitor.visit_generic_param_decl(for_);
    }

    walk_list!(visitor, visit_where_clause, &t.where_clauses);

    walk_map!(visitor, &t.methods);
}

pub fn walk_impl<'a, V: Visitor<'a>>(visitor: &mut V, i: &'a Impl) {
    visitor.visit_parse_type_inner(&i.name);
    if let Some(for_) = &i.for_ {
        visitor.visit_parse_type(for_);
    }
    walk_list!(visitor, visit_where_clause, &i.where_clauses);

    walk_map!(visitor, &i.methods);
}

pub fn walk_function_sig<'a, V: Visitor<'a>>(visitor: &mut V, function_sig: &'a FunctionSig) {
    visitor.visit_ident(&function_sig.name);
    visitor.visit_parse_type(&function_sig.sig);
    walk_list!(visitor, visit_where_clause, &function_sig.where_clauses);
}

pub fn walk_function_decl<'a, V: Visitor<'a>>(visitor: &mut V, function_decl: &'a FunctionDecl) {
    visitor.visit_ident(&function_decl.name);
    visitor.visit_lambda_decl(&function_decl.lambda);
}

pub fn walk_ident_or_type<'a, V: Visitor<'a>>(visitor: &mut V, ident: &'a IdentOrType) {
    match ident {
        IdentOrType::Ident(ident) => visitor.visit_ident(ident),
        IdentOrType::Type(ty) => visitor.visit_parse_type(ty),
    }
}

pub fn walk_path<'a, V: Visitor<'a>>(visitor: &mut V, path: &'a Path) {
    match path {
        Path::Ident(ident) => visitor.visit_identifier_path(ident),
        Path::Type(ty) => visitor.visit_type_path(ty),
    }
}

pub fn walk_identifier_path<'a, V: Visitor<'a>>(
    visitor: &mut V,
    identifier_path: &'a IdentifierPath,
) {
    walk_list!(visitor, visit_ident, &identifier_path.path);
}

pub fn walk_type_path<'a, V: Visitor<'a>>(visitor: &mut V, identifier_path: &'a TypePath) {
    walk_list!(visitor, visit_ident, &identifier_path.path);
}

pub fn walk_ident<'a, V: Visitor<'a>>(visitor: &mut V, identifier: &'a Ident) {
    visitor.visit_name(&identifier.name);
}

pub fn walk_ident_or_number<'a, V: Visitor<'a>>(visitor: &mut V, ident: &'a IdentOrNumber) {
    match ident {
        IdentOrNumber::Ident(ident) => visitor.visit_ident(ident),
        IdentOrNumber::Number(num) => visitor.visit_primitive(num),
    }
}

pub fn walk_block<'a, V: Visitor<'a>>(visitor: &mut V, body: &'a Block) {
    walk_list!(visitor, visit_statement, &body.statements);
}

pub fn walk_statement<'a, V: Visitor<'a>>(visitor: &mut V, statement: &'a Statement) {
    match &statement {
        Statement::Assignment(assign) => visitor.visit_assignment(assign),
        Statement::Expression(expr) => visitor.visit_expression(expr),
        Statement::Return(expr) => {
            if let Some(expr) = expr {
                visitor.visit_expression(expr);
            }
        }
        Statement::Continue(expr) => {
            if let Some(expr) = expr {
                visitor.visit_expression(expr);
            }
        }
        Statement::Break(expr) => {
            if let Some(expr) = expr {
                visitor.visit_expression(expr);
            }
        }
    }
}

pub fn walk_assignment<'a, V: Visitor<'a>>(visitor: &mut V, assign: &'a Assignment) {
    visitor.visit_assignment_l_h_s(&assign.lhs);
    visitor.visit_expression(&assign.rhs);
}

pub fn walk_assignment_l_h_s<'a, V: Visitor<'a>>(visitor: &mut V, assign: &'a AssignmentLHS) {
    match assign {
        AssignmentLHS::Expression(expr) => visitor.visit_unary_expr(expr),
        AssignmentLHS::Pattern {
            pattern,
            type_annotation,
        } => {
            visitor.visit_pattern(pattern);

            if let Some(type_annotation) = type_annotation {
                visitor.visit_parse_type(type_annotation);
            }
        }
    }
}

pub fn walk_if<'a, V: Visitor<'a>>(visitor: &mut V, r#if: &'a If) {
    visitor.visit_condition(&r#if.condition);

    visitor.visit_block(&r#if.then);

    if let Some(r#else) = &r#if.else_ {
        visitor.visit_else(r#else);
    }
}

pub fn walk_else<'a, V: Visitor<'a>>(visitor: &mut V, r#else: &'a Else) {
    match r#else {
        Else::If(if_) => visitor.visit_if(if_),
        Else::Block(block) => visitor.visit_block(block),
    }
}

pub fn walk_expression<'a, V: Visitor<'a>>(visitor: &mut V, expr: &'a Expression) {
    match &expr {
        Expression::BinopExpr(unary, operator, expr) => {
            visitor.visit_unary_expr(unary);
            visitor.visit_operator(operator);
            visitor.visit_expression(expr);
        }
        Expression::UnaryExpr(unary) => visitor.visit_unary_expr(unary),
        Expression::CastExpr(inner, _ty) => visitor.visit_expression(inner),
        Expression::Range(range) => {
            if let Some(start) = &range.start {
                visitor.visit_expression(start);
            }
            if let Some(end) = &range.end {
                visitor.visit_expression(end);
            }
        }
    }
}

pub fn walk_condition<'a, V: Visitor<'a>>(visitor: &mut V, condition: &'a Condition) {
    if let Some(pattern) = &condition.pattern {
        visitor.visit_pattern(pattern);
    }

    visitor.visit_expression(&condition.expression);
}

pub fn walk_instance<'a, V: Visitor<'a>>(visitor: &mut V, s: &'a Instance) {
    visitor.visit_type_path(&s.name);

    walk_map!(visitor, &s.fields);
}

pub fn walk_unary_expr<'a, V: Visitor<'a>>(visitor: &mut V, unary: &'a UnaryExpr) {
    match unary {
        UnaryExpr::PrimaryExpr(primary) => visitor.visit_primary_expr(primary),
        UnaryExpr::UnaryExpr(op, unary) => {
            visitor.visit_operator(op);
            visitor.visit_unary_expr(unary);
        }
    }
}

pub fn walk_primary_expr<'a, V: Visitor<'a>>(visitor: &mut V, primary: &'a PrimaryExpr) {
    visitor.visit_operand(&primary.operand);

    if let Some(secondaries) = &primary.secondaries {
        walk_list!(visitor, visit_secondary_expr, secondaries);
    }
}

pub fn walk_secondary_expr<'a, V: Visitor<'a>>(visitor: &mut V, secondary: &'a SecondaryExpr) {
    match secondary {
        SecondaryExpr::Arguments(args) => {
            walk_list!(visitor, visit_argument, args);
        }
        SecondaryExpr::Indice(expr) => {
            visitor.visit_expression(expr);
        }
        SecondaryExpr::Dot(expr) => {
            visitor.visit_ident_or_number(expr);
        }
        SecondaryExpr::Interogation => {}
    }
}

pub fn walk_operator<'a, V: Visitor<'a>>(_visitor: &mut V, _operator: &'a Operator) {}

pub fn walk_operand<'a, V: Visitor<'a>>(visitor: &mut V, operand: &'a Operand) {
    match &operand {
        Operand::Literal(l) => visitor.visit_literal(l),
        Operand::Ident(i) => visitor.visit_identifier_path(i),
        Operand::CallHole(_) => {}
        Operand::SelfIdent(i) => visitor.visit_ident(i),
        Operand::Instance(s) => visitor.visit_instance(s),
        Operand::NativeOperator(n) => visitor.visit_native_operator(n),
        Operand::LambdaDecl(l) => visitor.visit_lambda_decl(l),
        Operand::Tuple(t) => visitor.visit_tuple(t),
        Operand::If(i) => visitor.visit_if(i),
        Operand::Loop(l) => visitor.visit_loop(l),
        Operand::Expression(e) => visitor.visit_expression(e),
        Operand::Match(m) => visitor.visit_match(m),
        Operand::Unsafe(block, _) => visitor.visit_block(block),
        Operand::Open(open) => {
            visitor.visit_expression(&open.source);
            visitor.visit_ident(&open.witness);
            visitor.visit_ident(&open.value);
            visitor.visit_block(&open.body);
        }
    }
}

pub fn walk_match<'a, V: Visitor<'a>>(visitor: &mut V, m: &'a Match) {
    visitor.visit_expression(&m.expr);

    walk_list!(visitor, visit_match_arm, &m.arms);
}

pub fn walk_match_arm<'a, V: Visitor<'a>>(visitor: &mut V, m: &'a MatchArm) {
    visitor.visit_pattern(&m.pattern);
    visitor.visit_block(&m.body);
}

pub fn walk_pattern<'a, V: Visitor<'a>>(visitor: &mut V, p: &'a Pattern) {
    if let Some(ident) = &p.binding {
        visitor.visit_ident(ident);
    }

    visitor.visit_pattern_kind(&p.kind);
}

pub fn walk_pattern_kind<'a, V: Visitor<'a>>(visitor: &mut V, m: &'a PatternKind) {
    match m {
        PatternKind::Ident(ident) => visitor.visit_ident_pattern(ident),
        PatternKind::Literal(l) => visitor.visit_literal(l),
        PatternKind::Tuple(patterns) => walk_list!(visitor, visit_match_pattern, patterns),
        PatternKind::Array(patterns) => walk_list!(visitor, visit_array_pattern, patterns),
        PatternKind::Instance(e) => visitor.visit_instance_pattern(e),
        PatternKind::Nested(p) => visitor.visit_pattern(p),
        PatternKind::Wildcard => {}
        PatternKind::Reference { pattern, .. } => visitor.visit_pattern(pattern),
    }
}

pub fn walk_ident_pattern<'a, V: Visitor<'a>>(visitor: &mut V, i: &'a IdentPattern) {
    visitor.visit_ident(&i.name);
}

pub fn walk_instance_pattern<'a, V: Visitor<'a>>(visitor: &mut V, e: &'a InstancePattern) {
    visitor.visit_type_path(&e.name);
    visitor.visit_fields_pattern_or_arguments_pattern(&e.args);
}

pub fn walk_fields_pattern_or_arguments_pattern<'a, V: Visitor<'a>>(
    visitor: &mut V,
    f: &'a FieldsPatternOrArgumentsPattern,
) {
    match f {
        FieldsPatternOrArgumentsPattern::Fields(fields) => {
            walk_list!(visitor, visit_field_pattern, fields);
        }
        FieldsPatternOrArgumentsPattern::Arguments(args) => {
            walk_list!(visitor, visit_argument, args);
        }
    }
}

pub fn walk_field_pattern<'a, V: Visitor<'a>>(visitor: &mut V, f: &'a FieldPattern) {
    visitor.visit_ident(&f.name);
    visitor.visit_pattern(&f.pattern);
}

pub fn walk_array_pattern<'a, V: Visitor<'a>>(visitor: &mut V, a: &'a ArrayPattern) {
    match a {
        ArrayPattern::Pattern(p) => visitor.visit_pattern(p),
        ArrayPattern::Rest(ident) => visitor.visit_ident_pattern(ident),
    }
}

pub fn walk_tuple<'a, V: Visitor<'a>>(visitor: &mut V, t: &'a Tuple) {
    walk_list!(visitor, visit_expression, &t.elements);
}

pub fn walk_native_operator<'a, V: Visitor<'a>>(visitor: &mut V, n: &'a NativeOperator) {
    visitor.visit_name(&n.name);
}

pub fn walk_argument<'a, V: Visitor<'a>>(visitor: &mut V, argument: &'a Argument) {
    visitor.visit_expression(&argument.arg);
}

pub fn walk_literal<'a, V: Visitor<'a>>(visitor: &mut V, literal: &'a Literal) {
    match &literal.kind {
        LiteralKind::Number(n) => visitor.visit_primitive(n),
        LiteralKind::Float(f) => visitor.visit_primitive(f),
        LiteralKind::String(s) => visitor.visit_primitive(s),
        LiteralKind::Bool(b) => visitor.visit_primitive(b),
        LiteralKind::Array(arr) => visitor.visit_array(arr),
        LiteralKind::ArrayRepeat { value, .. } => visitor.visit_expression(value),
        LiteralKind::Char(c) => visitor.visit_primitive(c),
    }
}

pub fn walk_array<'a, V: Visitor<'a>>(visitor: &mut V, arr: &'a Array) {
    walk_list!(visitor, visit_expression, &arr.elements);
}

pub fn walk_parse_type<'a, V: Visitor<'a>>(visitor: &mut V, ty: &'a ParseType) {
    match ty {
        ParseType::Object(object) => {
            for ty in object.types() {
                visitor.visit_parse_type(ty);
            }
            for qualifier in &object.qualifiers {
                if let ObjectQualifier::Binding { member, .. } = qualifier {
                    visitor.visit_ident(member);
                }
            }
        }
        ParseType::Slice(ty) => {
            visitor.visit_parse_type(ty);
        }
        ParseType::Array { inner, .. } => {
            visitor.visit_parse_type(inner);
        }
        ParseType::Tuple(tys) => {
            walk_list!(visitor, visit_parse_type, tys);
        }
        ParseType::Function(tys) => {
            walk_list!(visitor, visit_parse_type, tys);
        }
        ParseType::Type(ident) => {
            visitor.visit_parse_type_inner(ident);
        }
        ParseType::Application(application) => visitor.visit_type_application(application),
        ParseType::Lambda(lambda) => visitor.visit_type_lambda(lambda),
        ParseType::Hole(hole) => visitor.visit_type_hole(hole),
        ParseType::Associated { base, member } => {
            visitor.visit_parse_type_inner(base);
            visitor.visit_ident(member);
        }
        ParseType::Reference { is_mut: _, pointee } => {
            visitor.visit_parse_type(pointee);
        }
        ParseType::Pointer(pointee) => {
            visitor.visit_parse_type(pointee);
        }
        ParseType::Unit(_) => {}
    }
    // walk_list!(visitor, visit_parse_type_inner, &ty.inners);
}

pub fn walk_parse_type_inner<'a, V: Visitor<'a>>(visitor: &mut V, ty: &'a ParseTypeInner) {
    visitor.visit_primitive(&ty.name);
    walk_list!(visitor, visit_parse_type, &ty.generics);
}

pub fn walk_generic_param_decl<'a, V: Visitor<'a>>(visitor: &mut V, param: &'a GenericParamDecl) {
    visitor.visit_ident(&param.name);
    if let Some(bound) = &param.unsized_bound {
        visitor.visit_parse_type(bound);
    }
    if let Some(kind) = &param.kind {
        walk_list!(visitor, visit_parse_type, &kind.args);
    }
}

pub fn walk_type_application<'a, V: Visitor<'a>>(
    visitor: &mut V,
    application: &'a TypeApplication,
) {
    visitor.visit_parse_type(&application.constructor);
    walk_list!(visitor, visit_parse_type, &application.args);
}

pub fn walk_type_lambda<'a, V: Visitor<'a>>(visitor: &mut V, lambda: &'a TypeLambda) {
    walk_list!(visitor, visit_generic_param_decl, &lambda.params);
    visitor.visit_parse_type(&lambda.body);
}

pub fn walk_type_hole<'a, V: Visitor<'a>>(_visitor: &mut V, _hole: &'a TypeHole) {}

pub fn walk_where_clause<'a, V: Visitor<'a>>(visitor: &mut V, clause: &'a WhereClause) {
    visitor.visit_parse_type(&clause.subject);
    if let Some(bound) = &clause.trait_bound {
        visitor.visit_parse_type(bound);
    }
}

pub fn walk_loop<'a, V: Visitor<'a>>(visitor: &mut V, loop_: &'a Loop) {
    match loop_ {
        Loop::For(pattern, condition, block, _) => {
            visitor.visit_pattern(pattern);
            visitor.visit_expression(condition);
            visitor.visit_block(block);
        }
        Loop::While(condition, block, _) => {
            visitor.visit_condition(condition);
            visitor.visit_block(block);
        }
        Loop::Loop(block, _) => visitor.visit_block(block),
    }
}

pub fn walk_lambda_decl<'a, V: Visitor<'a>>(visitor: &mut V, lambda: &'a LambdaDecl) {
    walk_list!(visitor, visit_ident_or_type, &lambda.parameters);
    visitor.visit_block(&lambda.body);
}

pub fn walk_enum_decl<'a, V: Visitor<'a>>(visitor: &mut V, e: &'a EnumDecl) {
    visitor.visit_parse_type_inner(&e.name);
    walk_list!(visitor, visit_enum_variant, &e.variants);
}

pub fn walk_enum_variant<'a, V: Visitor<'a>>(visitor: &mut V, e: &'a EnumVariant) {
    visitor.visit_parse_type_inner(&e.name);
    visitor.visit_named_fields_or_types_list(&e.fields);
}

pub fn walk_named_fields_or_types_list<'a, V: Visitor<'a>>(
    visitor: &mut V,
    n: &'a NamedFieldsOrTypesList,
) {
    match n {
        NamedFieldsOrTypesList::NamedFields(fields) => {
            walk_list!(visitor, visit_struct_decl_field, fields);
        }
        NamedFieldsOrTypesList::TypesList(types) => {
            walk_list!(visitor, visit_parse_type, types);
        }
    }
}

pub fn walk_trait_decl<'a, V: Visitor<'a>>(visitor: &mut V, t: &'a TraitDecl) {
    visitor.visit_parse_type_inner(&t.name);

    walk_list!(visitor, visit_generic_param_decl, &t.generic_params);

    if let Some(for_) = &t.for_ {
        visitor.visit_generic_param_decl(for_);
    }

    for associated_type in &t.associated_types {
        visitor.visit_ident(&associated_type.name);
    }

    walk_map!(visitor, &t.methods);

    for (k, v) in &t.signatures {
        visitor.visit_ident(k);
        visitor.visit_function_sig(v);
    }
}

pub fn walk_macro_decl<'a, V: Visitor<'a>>(_visitor: &mut V, _m: &'a MacroDecl) {}

pub fn walk_macro_invoc<'a, V: Visitor<'a>>(_visitor: &mut V, _m: &'a MacroInvoc) {}

pub fn walk_program<'a, V: Visitor<'a>>(visitor: &mut V, program: &'a Program) {
    visitor.visit_module(&program.module);
}

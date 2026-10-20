use std::collections::{BTreeSet, HashSet};

use super::*;
use crate::{ast::*, Config};
use crate::lexer::interpretation::layout::LayoutView;

const LIMITS: ScanLimits = crate::macro_expansion::token_tree::MACRO_LIMITS;

fn marked(text: &str, context: u32) -> LosslessStream {
    let mut stream = crate::macro_expansion::token_tree::source(text);
    for run in &mut stream.runs { run.context = SyntaxContext(context); }
    stream.anchor.context = SyntaxContext(context);
    stream
}

fn names(stream: &LosslessStream) -> Vec<Ident> {
    struct Names(Vec<Ident>);
    impl<'a> crate::ast::visit::Visitor<'a> for Names {
        fn visit_ident(&mut self, name: &'a Ident) { self.0.push(name.clone()); }
    }
    let module = crate::parser::parse_lossless_source(stream, &Config::default(), None).unwrap();
    let mut visitor = Names(Vec::new());
    crate::ast::visit::walk_module(&mut visitor, &module);
    visitor.0
}

#[test]
fn macro_a_11_ident_identity_agrees_for_equality_hash_and_order_not_locations() {
    let first = names(&marked("value = -> 42\n", 7)).remove(0);
    let second = names(&marked("value = -> 42\n", 8)).remove(0);
    assert_ne!(first, second);
    assert_eq!(HashSet::from([first.clone(), second.clone()]).len(), 2);
    assert_eq!(BTreeSet::from([first.clone(), second]).len(), 2);
    let mut moved = first.clone();
    moved.span = crate::lexer::Span::new("/elsewhere.rk".into(), 100, 105);
    assert_eq!(first, moved);
    assert_eq!(first.cmp(&moved), std::cmp::Ordering::Equal);
}

#[test]
fn macro_a_11_parser_names_paths_and_operators_keep_interpreted_contexts() {
    let stream = marked("main = value -> pkg::helper value + value\n", 11);
    let parsed_names = names(&stream);
    assert!(parsed_names.len() >= 5);
    assert!(parsed_names.iter().all(|name| name.context == SyntaxContext(11)));
    let module = crate::parser::parse_lossless_source(&stream, &Config::default(), None).unwrap();
    struct Operators(Vec<Operator>);
    impl<'a> crate::ast::visit::Visitor<'a> for Operators {
        fn visit_operator(&mut self, operator: &'a Operator) { self.0.push(operator.clone()); }
    }
    let mut visitor = Operators(Vec::new());
    crate::ast::visit::walk_module(&mut visitor, &module);
    assert_eq!(visitor.0.len(), 1);
    assert_eq!(visitor.0[0].context, SyntaxContext(11));
}

#[test]
fn macro_a_11_classification_slicing_grouping_and_expression_forwarding_do_not_remint_contexts() {
    let original = marked("(pkg::value)", 17);
    let groups = crate::lexer::groups::BalancedGroups::parse(&original,
        crate::lexer::groups::GroupLimits { max_depth: 8, max_groups: 8 }).unwrap();
    assert_eq!(groups.records().len(), 1);
    assert_eq!(groups.group(0).unwrap().inner(LIMITS).unwrap().binding_context(0..10).unwrap(), SyntaxContext(17));
    let slice = original.slice(1..11, LIMITS).unwrap();
    let forwarded = slice.expression_fragment(0, LIMITS).unwrap();
    let expression = crate::parser::parse_lossless_expression(&forwarded, &Config::default(), None).unwrap();
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { panic!("primary") };
    let Operand::Expression(expression) = primary.operand else { panic!("opaque expression") };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = *expression else { panic!("primary") };
    let Operand::Ident(path) = primary.operand else { panic!("path") };
    assert!(path.path.iter().all(|part| matches!(part, IdentOrType::Ident(name) if name.context == SyntaxContext(17))));
    let mut view = LayoutView::new(&original, false);
    while let Some(token) = view.next_token().unwrap() {
        if let Some(context) = token.context { assert_eq!(token.token.context, context); }
    }
    assert_eq!(original.binding_context(1..11).unwrap(), SyntaxContext(17));
}

#[test]
fn macro_a_11_captures_and_nested_macro_paths_retain_their_own_contexts() {
    let head = marked("main = -> %pkg::outer ", 19);
    let input = marked("captured\n", 23);
    let combined = head.join(&input, JoinBoundary::Separate, LIMITS).unwrap();
    let module = crate::parser::parse_lossless_source(&combined, &Config::default(), None).unwrap();
    let TopLevel::FunctionDecl(function) = &module.top_levels[0] else { panic!("function") };
    let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &function.lambda.body.statements[0] else { panic!("primary") };
    let Operand::MacroInvoc(call) = &primary.operand else { panic!("call") };
    assert!(call.path.path.iter().all(|part| matches!(part, IdentOrType::Ident(name) if name.context == SyntaxContext(19))));
    assert_eq!(call.args.binding_context(1..9).unwrap(), SyntaxContext(23));
}

#[test]
fn macro_a_11_ordinary_source_names_keep_root_context() {
    let parsed = names(&marked("main = value -> value\n", 0));
    assert!(parsed.len() >= 3);
    assert!(parsed.iter().all(|name| name.context == SyntaxContext::ROOT));
}

#[test]
fn macro_a_11_retained_macro_payloads_do_not_enlarge_every_expression_frame() {
    assert!(std::mem::size_of::<Operand>() < std::mem::size_of::<MacroInvoc>());
}

#[test]
fn macro_a_11_declarative_forwarding_keeps_captures_separate_from_template_names() {
    let definition = marked("macro duplicate\n    $name:ident =>\n        ($name, $name)\n", 31);
    let call = marked("main = -> %duplicate ", 37);
    let argument = marked("captured\n", 41);
    let stream = definition.join(&call, JoinBoundary::Separate, LIMITS).unwrap()
        .join(&argument, JoinBoundary::Separate, LIMITS).unwrap();
    let config = Config::default();
    let module = crate::parser::parse_lossless_source(&stream, &config, None).unwrap();
    let expanded = crate::macro_expansion::expand_macros_with_context(Program { module },
        &crate::macro_expansion::MacroExpansionContext::new(&config)).unwrap();
    struct Captured(Vec<Ident>);
    impl<'a> crate::ast::visit::Visitor<'a> for Captured {
        fn visit_ident(&mut self, name: &'a Ident) {
            if name.name == "captured" { self.0.push(name.clone()); }
        }
    }
    let mut visitor = Captured(Vec::new());
    crate::ast::visit::walk_module(&mut visitor, &expanded.module);
    assert_eq!(visitor.0.len(), 2);
    assert!(visitor.0.iter().all(|name| name.context == SyntaxContext(41)));
    assert_eq!(visitor.0[0], visitor.0[1]);
}

#[test]
fn macro_a_11_type_names_binders_and_associated_paths_keep_context() {
    let stream = marked("struct Holder A\n    item: A::Item\n    wrapped: Holder A\n", 43);
    let module = crate::parser::parse_lossless_source(&stream, &Config::default(), None).unwrap();
    struct Types(Vec<ParseTypeInner>, Vec<Ident>);
    impl<'a> crate::ast::visit::Visitor<'a> for Types {
        fn visit_parse_type_inner(&mut self, name: &'a ParseTypeInner) {
            self.0.push(name.clone());
            crate::ast::visit::walk_parse_type_inner(self, name);
        }
        fn visit_ident(&mut self, name: &'a Ident) { self.1.push(name.clone()); }
    }
    let mut visitor = Types(Vec::new(), Vec::new());
    crate::ast::visit::walk_module(&mut visitor, &module);
    assert!(visitor.0.len() >= 4);
    assert!(visitor.0.iter().all(|name| name.context == SyntaxContext(43)));
    assert!(visitor.1.iter().any(|name| name.name == "Item" && name.context == SyntaxContext(43)));
    let TopLevel::StructDecl(declaration) = &module.top_levels[0] else { panic!("struct") };
    assert_eq!(declaration.generic_params[0].name.context, SyntaxContext(43));
}

#[test]
fn macro_a_11_reclassification_keeps_punctuation_operator_and_self_contexts() {
    for text in ["^", "~", "&", "+"] {
        let stream = marked(text, 47);
        let input = crate::parser::source_cursor::SourceInput::new(&stream, false,
            crate::parser::source_cursor::SourceLimits { max_entries: 64, max_branches: 4 }).unwrap();
        let config = Config::default();
        let (cursor, _) = crate::parser::indent(crate::parser::ParseCtx::from_source(&input, &config)).unwrap();
        let (_, operator) = crate::parser::operator(cursor).unwrap();
        assert_eq!(operator.context, SyntaxContext(47));
    }
    let stream = marked("main = -> @\n", 47);
    assert!(names(&stream).iter().any(|name| name.name == "self" && name.context == SyntaxContext(47)));
}

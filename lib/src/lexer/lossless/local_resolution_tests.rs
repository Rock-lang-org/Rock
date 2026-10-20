use super::*;
use crate::ast;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, HirVarTarget};
use crate::ids::HirLocalId;
use crate::lower::Lowerer;

// Synthetic authorities remain confined to lossless tests until A.14 allocates
// production contexts. Slice one snapshot so diagnostic positions stay intact.
fn module(parts: &[(&str, u32)]) -> ast::Module {
    let source = crate::macro_expansion::token_tree::source(
        &parts.iter().map(|(text, _)| *text).collect::<String>(),
    );
    let limits = crate::macro_expansion::token_tree::MACRO_LIMITS;
    let mut offset = 0;
    let mut result = source.slice(0..0, limits).unwrap();
    for (text, context) in parts {
        let mut piece = source.slice(offset..offset + text.len(), limits).unwrap();
        for run in &mut piece.runs { run.context = SyntaxContext(*context); }
        piece.anchor.context = SyntaxContext(*context);
        result = result.join(&piece, JoinBoundary::Separate, limits).unwrap();
        offset += text.len();
    }
    crate::parser::parse_lossless_source(&result, &crate::Config::default(), None).unwrap()
}

fn body(parts: &[(&str, u32)]) -> HirBlock {
    let module = module(parts);
    let ast::TopLevel::FunctionDecl(function) = &module.top_levels[0] else { panic!("function") };
    let mut lowerer = Lowerer::new_for_test();
    let body = lowerer.with_test_body_context(|lowerer| lowerer.lower_block(&function.lambda.body));
    assert!(lowerer.diagnostics.is_empty(), "{:?}", lowerer.diagnostics.errors());
    body
}

fn binding(stmt: &HirStmt) -> HirLocalId {
    let HirStmt::Let { local_id, .. } = stmt else { panic!("binding: {stmt:?}") };
    *local_id
}

fn expression(stmt: &HirStmt) -> &HirExpr {
    let HirStmt::Expr(expr) = stmt else { panic!("expression: {stmt:?}") };
    expr
}

fn local(expr: &HirExpr) -> HirLocalId {
    let HirExprKind::ResolvedVar(reference) = &expr.kind else { panic!("local: {expr:?}") };
    let HirVarTarget::Local(id) = reference.target else { panic!("local target") };
    id
}

#[test]
fn macro_a_12_same_spelling_contexts_bind_and_reassign_independently() {
    let body = body(&[
        ("main = ->\n    mut value = 1\n", 0),
        ("    mut value = 2\n    value = 3\n    value\n", 7),
        ("    value = 4\n    value\n", 0),
    ]);
    let caller = binding(&body.stmts[0]);
    let generated = binding(&body.stmts[1]);
    assert_ne!(caller, generated);
    let HirExprKind::Assign(lhs, _) = &expression(&body.stmts[2]).kind else { panic!("assignment") };
    assert_eq!(local(lhs), generated);
    assert_eq!(local(expression(&body.stmts[3])), generated);
    let HirExprKind::Assign(lhs, _) = &expression(&body.stmts[4]).kind else { panic!("assignment") };
    assert_eq!(local(lhs), caller);
    assert_eq!(local(expression(&body.stmts[5])), caller);
}

#[test]
fn macro_a_12_tuple_destructuring_and_inner_shadow_keep_caller_owner() {
    let body = body(&[
        ("main = ->\n    value = 1\n", 0),
        ("    (value, other) = (2, 3)\n    if true\n        value: I64 = 4\n        value\n    value\n", 9),
        ("    value\n", 0),
    ]);
    let caller = binding(&body.stmts[0]);
    let generated = binding(&body.stmts[2]);
    let HirExprKind::If { then_branch, .. } = &expression(&body.stmts[4]).kind else { panic!("if") };
    let shadow = binding(&then_branch.stmts[0]);
    assert_ne!(shadow, generated);
    assert_eq!(local(expression(&then_branch.stmts[1])), shadow);
    assert_eq!(local(expression(&body.stmts[5])), generated);
    assert_eq!(local(expression(&body.stmts[6])), caller);
}

#[test]
fn macro_a_12_lambda_parameter_does_not_hide_different_context_capture() {
    let body = body(&[
        ("main = ->\n    value = 1\n    f = ", 0),
        ("value ->\n        value\n", 11),
        ("        value\n        value\n", 0),
    ]);
    let outer = binding(&body.stmts[0]);
    let HirStmt::Let { value, .. } = &body.stmts[1] else { panic!("lambda binding") };
    let HirExprKind::Lambda { params, body, captures } = &value.kind else { panic!("lambda") };
    assert_eq!(local(expression(&body.stmts[0])), params[0].local_id);
    assert_eq!(local(expression(&body.stmts[1])), outer);
    assert_eq!(captures.len(), 1, "duplicated references capture one owner");
    assert_eq!(captures[0].local_id, outer);
}

#[test]
fn macro_a_12_same_spelling_captures_are_distinct_and_mutable_reference_keeps_owner() {
    let body = body(&[
        ("main = ->\n    mut value = 1\n    loan = &mut value\n", 0),
        ("    value = 2\n    loan = &value\n", 13),
        ("    f = ->\n        loan\n", 0),
        ("        loan\n        loan\n", 13),
    ]);
    let caller_loan = binding(&body.stmts[1]);
    let generated_loan = binding(&body.stmts[3]);
    let HirStmt::Let { value, .. } = &body.stmts[4] else { panic!("lambda binding") };
    let HirExprKind::Lambda { captures, .. } = &value.kind else { panic!("lambda") };
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[0].local_id, caller_loan);
    assert_eq!(captures[0].kind, crate::hir::HirClosureCaptureKind::MutableBorrow);
    assert_eq!(captures[1].local_id, generated_loan);
    assert_eq!(captures[1].kind, crate::hir::HirClosureCaptureKind::SharedBorrow);
}

#[test]
fn macro_a_12_match_destructuring_uses_contextual_pattern_names() {
    let body = body(&[
        ("main = ->\n    value = 1\n    match (2, 3)\n", 0),
        ("        (value, other) =>\n            value\n", 17),
        ("            value\n", 0),
    ]);
    let caller = binding(&body.stmts[0]);
    let HirExprKind::Match { arms, .. } = &expression(&body.stmts[1]).kind else { panic!("match") };
    let crate::hir::HirPattern::Tuple(patterns) = &arms[0].pattern else { panic!("tuple") };
    let crate::hir::HirPattern::Binding { local_id, .. } = patterns[0] else { panic!("binding") };
    assert_ne!(caller, local_id);
    assert_eq!(local(expression(&arms[0].body.stmts[0])), local_id);
    assert_eq!(local(expression(&arms[0].body.stmts[1])), caller);
}

#[test]
fn macro_a_12_context_mismatch_never_falls_back_to_root_local() {
    let module = module(&[("main = ->\n    value = 1\n", 0), ("    value\n", 23)]);
    let ast::TopLevel::FunctionDecl(function) = &module.top_levels[0] else { panic!("function") };
    let mut lowerer = Lowerer::new_for_test();
    lowerer.with_test_body_context(|lowerer| lowerer.lower_block(&function.lambda.body));
    assert!(!lowerer.diagnostics.is_empty());
}

#[test]
fn macro_a_12_function_parameters_retain_context_through_header_collection() {
    let module = module(&[
        ("identity = ", 0), ("value -> value\n", 29),
        ("main = -> identity 42\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let function = partial.functions.values().find(|function| function.name == "identity").unwrap();
    assert_eq!(local(expression(&function.body.stmts[0])), function.params[0].local_id);
}

#[test]
fn macro_a_12_curried_and_nested_lambdas_capture_canonical_ids() {
    let body = body(&[
        ("main = ->\n    value = 1\n    f = ", 0),
        ("value, other ~>\n        value\n", 31),
        ("        value\n", 0),
    ]);
    let outer = binding(&body.stmts[0]);
    let HirStmt::Let { value, .. } = &body.stmts[1] else { panic!("lambda binding") };
    let HirExprKind::Lambda { params, body, captures } = &value.kind else { panic!("outer lambda") };
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].local_id, outer);
    let HirExprKind::Lambda { body, captures, .. } = &expression(&body.stmts[0]).kind else { panic!("inner lambda") };
    assert_eq!(local(expression(&body.stmts[0])), params[0].local_id);
    assert_eq!(local(expression(&body.stmts[1])), outer);
    assert_eq!(captures.len(), 2);
    assert!(captures.iter().any(|capture| capture.local_id == params[0].local_id));
    assert!(captures.iter().any(|capture| capture.local_id == outer));
}

#[test]
fn macro_a_12_array_loop_binding_does_not_hide_caller_local() {
    let body = body(&[
        ("main = ->\n    value = 1\n    for ", 0),
        ("value", 37),
        (" in [2, 3]\n", 0),
        ("        value\n", 37),
        ("        value\n", 0),
    ]);
    let caller = binding(&body.stmts[0]);
    let HirExprKind::For { local_id, body, .. } = &expression(&body.stmts[1]).kind else { panic!("for") };
    assert_ne!(*local_id, caller);
    assert_eq!(local(expression(&body.stmts[0])), *local_id);
    assert_eq!(local(expression(&body.stmts[1])), caller);
}

#[test]
fn macro_a_12_impl_receiver_marker_and_parameter_are_independent_of_method_name() {
    let module = module(&[
        ("struct Holder\n    value: I64\nimpl Holder\n    ", 0),
        ("@", 41), ("read = ", 0), ("argument -> (self.value, argument)\n", 41),
        ("main = -> 0\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let method = partial.impls.values().next().unwrap().methods.get("read").unwrap();
    let HirExprKind::TupleLiteral(elements) = &expression(&method.body.stmts[0]).kind else { panic!("tuple") };
    let HirExprKind::FieldAccess(receiver, _, _) = &elements[0].kind else { panic!("field") };
    let HirExprKind::Deref(receiver) = &receiver.kind else { panic!("borrowed receiver") };
    assert_eq!(local(receiver), method.params[0].local_id);
    assert_eq!(local(&elements[1]), method.params[1].local_id);
}

#[test]
fn macro_a_12_trait_default_receiver_context_is_independent_of_method_name() {
    let module = module(&[
        ("trait Read\n    ", 0), ("@", 43), ("read: &Self\n    ", 0),
        ("@", 43), ("read = -> ", 0),
        ("self\n", 43), ("main = -> 0\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let method = partial.traits.values().next().unwrap().methods.get("read").unwrap();
    assert_eq!(local(expression(&method.body.stmts[0])), method.params[0].local_id);
}

fn checked_mir(parts: &[(&str, u32)]) -> Result<crate::mir::MirProgram, crate::diagnostic::Diagnostics> {
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module: module(parts) }).unwrap();
    let resolved = crate::infer::finalize(partial).unwrap();
    let mut mono = crate::mono::monomorphize(resolved.program, resolved.source_map)?;
    let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    crate::mir::borrowck::BorrowChecker::run(&mir)?;
    Ok(mir)
}

#[test]
fn macro_a_12_mutable_capture_with_generated_collision_passes_ownership_pipeline() {
    checked_mir(&[
        ("main = !->\n    mut value = 1\n", 0),
        ("    value = 2\n", 47),
        ("    mut f = ->\n        value = 3\n    f!\n", 0),
        ("    value\n", 47),
    ]).unwrap();
}

#[test]
fn macro_a_12_generated_collision_does_not_authorize_conflicting_caller_borrow() {
    let errors = checked_mir(&[
        ("main = !->\n    mut value = 1\n    loan = &mut value\n", 0),
        ("    mut value = 2\n", 53),
        ("    value = 3\n    *loan = 4\n", 0),
    ]).unwrap_err();
    assert!(errors.0.iter().any(|error| error.code == Some(crate::diagnostic::DiagnosticCode::Borrow)), "{errors:?}");
}

#[test]
fn macro_a_12_distinct_context_owner_write_preserves_independent_caller_loan() {
    checked_mir(&[
        ("main = !->\n    mut value = 1\n    loan = &mut value\n", 0),
        ("    mut value = 2\n    value = 3\n", 53),
        ("    *loan = 4\n", 0),
    ]).unwrap();
}

#[test]
fn macro_a_12_same_spelling_captures_remain_distinct_in_mir() {
    let mir = checked_mir(&[
        ("main = !->\n    value = 1\n", 0),
        ("    value = 2\n", 67),
        ("    f = ->\n        value\n", 0),
        ("        value\n", 67),
        ("    f!\n", 0),
    ]).unwrap();
    let closure = mir.functions().map(|(_, function)| function)
        .find(|function| function.closure_captures.len() == 2).expect("two-owner closure");
    assert_ne!(closure.closure_captures[0].local, closure.closure_captures[1].local);
    assert_eq!(closure.closure_captures[0].name, closure.closure_captures[1].name);
}

#[test]
fn macro_a_12_operator_flattening_keeps_context_for_local_callee() {
    let mut lowerer = Lowerer::new_for_test();
    let ty = crate::types::Type::function(vec![crate::types::Type::I64; 2], crate::types::Type::I64);
    lowerer.scope.define_local("%%".to_string(), ty.clone(), false, HirLocalId(1));
    lowerer.scope.define_local(crate::lower::scope::LocalName::new("%%".into(), SyntaxContext(59)), ty, false, HirLocalId(2));
    lowerer.infix_precedence.insert("%%".into(), 5);
    let number = |value| ast::UnaryExpr::PrimaryExpr(ast::PrimaryExpr {
        operand: ast::Operand::Literal(ast::Literal { kind: ast::LiteralKind::Number(value), span: crate::lexer::Span::test() }),
        secondaries: None,
        type_annotation: None,
    });
    let expr = ast::Expression::BinopExpr(number(1), ast::Operator {
        value: "%%".into(), context: SyntaxContext(59), span: crate::lexer::Span::test(),
    }, Box::new(ast::Expression::UnaryExpr(number(2))));
    let hir = lowerer.lower_expression(&expr);
    let HirExprKind::Call(callee, _, Some(crate::hir::HirCallTarget::Local(id))) = &hir.kind else { panic!("local call: {hir:?}") };
    assert_eq!(*id, HirLocalId(2));
    assert_eq!(local(callee), HirLocalId(2));
}

#[test]
fn macro_a_12_generated_local_does_not_remove_real_function_dependency() {
    let module = module(&[
        ("main = ->\n", 0), ("    helper = -> false\n", 61),
        ("    helper!\nhelper = -> 42\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let helper = partial.functions.values().find(|function| function.name == "helper").unwrap().id;
    let main = partial.functions.values().find(|function| function.name == "main").unwrap().id;
    let helper_component = partial.inference_sccs[&helper];
    let main_component = partial.inference_sccs[&main];
    assert_ne!(helper_component, main_component);
    assert!(partial.inference_scc_order.iter().position(|id| *id == helper_component).unwrap()
        < partial.inference_scc_order.iter().position(|id| *id == main_component).unwrap());
    crate::infer::finalize(partial).unwrap();
}

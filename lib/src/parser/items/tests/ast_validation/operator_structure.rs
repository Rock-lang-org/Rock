use super::*;

// ===========================================================================
// Operator chain structure
// ===========================================================================

#[test]
fn binop_chain_is_right_associative() {
    // `1 + 2 + 3` should parse as BinopExpr(1, +, BinopExpr(2, +, 3))
    let p = parse("main = -> 1 + 2 + 3\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    assert_eq!(unary_lit_number(lhs), 1);

    // rhs should be another BinopExpr(2, +, 3)
    let (lhs2, op2, rhs2) = as_binop(rhs);
    assert_eq!(op2, "+");
    assert_eq!(unary_lit_number(lhs2), 2);
    assert_eq!(lit_number(rhs2), 3);
}

#[test]
fn mixed_operators_form_right_recursive_chain() {
    // `1 + 2 * 3 - 4` → BinopExpr(1, +, BinopExpr(2, *, BinopExpr(3, -, 4)))
    let p = parse("main = -> 1 + 2 * 3 - 4\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    assert_eq!(unary_lit_number(lhs), 1);

    let (lhs2, op2, rhs2) = as_binop(rhs);
    assert_eq!(op2, "*");
    assert_eq!(unary_lit_number(lhs2), 2);

    let (lhs3, op3, rhs3) = as_binop(rhs2);
    assert_eq!(op3, "-");
    assert_eq!(unary_lit_number(lhs3), 3);
    assert_eq!(lit_number(rhs3), 4);
}

#[test]
fn comparison_operators_in_chain() {
    // `a == b` should be a single BinopExpr
    let p = parse("main = -> a == b\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "==");
    assert_eq!(unary_ident_name(lhs), "a");
    assert_eq!(ident_name(rhs), "b");
}

#[test]
fn four_level_operator_chain() {
    // `a + b + c + d + e` → 4 levels of right-nesting
    let p = parse("main = -> a + b + c + d + e\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, _, rhs) = as_binop(expr);
    assert_eq!(unary_ident_name(lhs), "a");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "b");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "c");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "d");

    // terminal
    assert_eq!(ident_name(rhs), "e");
}

// ===========================================================================
// Unary operators
// ===========================================================================

#[test]
fn unary_operator_wraps_inner_expression() {
    // `-x` should be UnaryExpr(-, PrimaryExpr(x))
    let p = parse("main = -> -x\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    match expr {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, inner)) => {
            assert_eq!(op.value, "-");
            match inner.as_ref() {
                UnaryExpr::PrimaryExpr(pe) => {
                    assert!(matches!(&pe.operand, Operand::Ident(_)));
                }
                other => panic!("expected PrimaryExpr inside unary, got {:?}", other),
            }
        }
        other => panic!("expected unary expression, got {:?}", other),
    }
}

#[test]
fn double_unary_is_single_stuck_operator() {
    // `!!x` is lexed as a single stuck operator "!!" applied to x
    let p = parse("main = -> !!x\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    match expr {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, inner)) => {
            assert_eq!(op.value, "!!");
            assert!(
                matches!(inner.as_ref(), UnaryExpr::PrimaryExpr(_)),
                "inner should be PrimaryExpr(x)"
            );
        }
        other => panic!("expected unary expression, got {:?}", other),
    }
}

#[test]
fn unary_before_binop() {
    // `-a + b` should be BinopExpr(UnaryExpr(-, a), +, b)
    let p = parse("main = -> -a + b\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    // LHS should be unary negation
    match lhs {
        UnaryExpr::UnaryExpr(unop, _inner) => assert_eq!(unop.value, "-"),
        other => panic!("expected unary LHS, got {:?}", other),
    }
    assert_eq!(ident_name(rhs), "b");
}

// ===========================================================================
// Function call argument binding
// ===========================================================================

#[test]
fn single_argument_binds_to_function() {
    // `foo 42` → foo with one Argument(42)
    let p = parse("main = -> foo 42\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    assert!(matches!(&pe.operand, Operand::Ident(_)));
    let secs = pe.secondaries.as_ref().expect("should have secondaries");
    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => {
            assert_eq!(args.len(), 1);
            assert_eq!(lit_number(&args[0].arg), 42);
        }
        other => panic!("expected Arguments, got {:?}", other),
    }
}

#[test]
fn multiple_arguments_bind_together() {
    // `foo 1, 2, 3` → foo with one Arguments([1, 2, 3])
    let p = parse("main = -> foo 1, 2, 3\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    let secs = pe.secondaries.as_ref().expect("should have secondaries");
    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => {
            assert_eq!(args.len(), 3);
            assert_eq!(lit_number(&args[0].arg), 1);
            assert_eq!(lit_number(&args[1].arg), 2);
            assert_eq!(lit_number(&args[2].arg), 3);
        }
        other => panic!("expected Arguments, got {:?}", other),
    }
}

#[test]
fn function_call_then_dot_structure() {
    // `foo.bar` → foo with Dot(bar)
    let p = parse("main = -> foo.bar\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Dot(IdentOrNumber::Ident(id)) => assert_eq!(id.name, "bar"),
        other => panic!("expected Dot(bar), got {:?}", other),
    }
}

#[test]
fn chained_dots_structure() {
    // `a.b.c.d` → one PrimaryExpr with 3 Dot secondaries
    let p = parse("main = -> a.b.c.d\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 3, "should have 3 dot secondaries");
    let names: Vec<&str> = secs
        .iter()
        .map(|s| match s {
            SecondaryExpr::Dot(IdentOrNumber::Ident(id)) => id.name.as_str(),
            other => panic!("expected Dot, got {:?}", other),
        })
        .collect();
    assert_eq!(names, vec!["b", "c", "d"]);
}

#[test]
fn index_expression_structure_deep() {
    // `arr[0]` → arr with Indice(0)
    let p = parse("main = -> arr[0]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Indice(inner) => {
            assert_eq!(lit_number(inner), 0);
        }
        other => panic!("expected Indice, got {:?}", other),
    }
}

#[test]
fn chained_index_expressions_structure() {
    // `arr[0][1]` → arr with 2 Indice secondaries
    let p = parse("main = -> arr[0][1]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 2);
    assert!(matches!(&secs[0], SecondaryExpr::Indice(_)));
    assert!(matches!(&secs[1], SecondaryExpr::Indice(_)));
}

#[test]
fn interleaved_dot_and_index() {
    // `a.b[0].c` → a with [Dot(b), Indice(0), Dot(c)]
    let p = parse("main = -> a.b[0].c\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 3, "secondaries: {:?}", secs);
    assert!(matches!(&secs[0], SecondaryExpr::Dot(IdentOrNumber::Ident(id)) if id.name == "b"));
    assert!(matches!(&secs[1], SecondaryExpr::Indice(_)));
    assert!(matches!(&secs[2], SecondaryExpr::Dot(IdentOrNumber::Ident(id)) if id.name == "c"));
}

#[test]
fn bang_call_has_no_arguments_structure() {
    // `foo!` → foo with Arguments([]) (bang = zero-arg call)
    let p = parse("main = -> foo!\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => assert!(args.is_empty(), "bang call should have 0 args"),
        other => panic!("expected Arguments, got {:?}", other),
    }
}

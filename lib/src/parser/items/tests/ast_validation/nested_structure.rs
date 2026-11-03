use super::*;

// ===========================================================================
// Expressions-as-values: if/match/loop used in assignment RHS
// ===========================================================================

#[test]
fn if_as_assignment_value_structure() {
    let p = parse("main = ->\n    x = if true then 1 else 2\n    x\n");
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // RHS should contain an If operand
    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::If(_)),
        "RHS should be an If, got {:?}",
        pe.operand
    );

    // Dig into the If
    if let Operand::If(if_expr) = &pe.operand {
        // Condition should be `true`
        let cond_expr = &if_expr.condition.expression;
        match as_operand(cond_expr) {
            Operand::Literal(lit) => assert_eq!(lit.kind, LiteralKind::Bool(true)),
            other => panic!("expected bool literal in condition, got {:?}", other),
        }

        // Then-block should have one statement yielding 1
        assert_eq!(if_expr.then.statements.len(), 1);
        assert_eq!(lit_number(as_expr(&if_expr.then.statements[0])), 1);

        // Else-block should yield 2
        match &if_expr.else_ {
            Some(Else::Block(block)) => {
                assert_eq!(block.statements.len(), 1);
                assert_eq!(lit_number(as_expr(&block.statements[0])), 2);
            }
            other => panic!("expected Else::Block, got {:?}", other),
        }
    }
}

#[test]
fn match_as_assignment_value_structure() {
    let input = r#"main = ->
    x = match 5
        0 => "zero"
        _ => "other"
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Match(_)),
        "RHS should be a Match"
    );

    if let Operand::Match(m) = &pe.operand {
        // Scrutinee should be 5
        assert_eq!(lit_number(&m.expr), 5);
        // Should have 2 arms
        assert_eq!(m.arms.len(), 2);
        // First arm pattern should be literal 0
        assert!(
            matches!(&m.arms[0].pattern.kind, PatternKind::Literal(lit) if lit.kind == LiteralKind::Number(0))
        );
        // Second arm should be wildcard
        assert!(matches!(&m.arms[1].pattern.kind, PatternKind::Wildcard));
    }
}

#[test]
fn loop_as_assignment_value_structure() {
    let input = r#"main = ->
    x = loop
        break 42
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Loop(_)),
        "RHS should be a Loop"
    );

    if let Operand::Loop(lp) = &pe.operand {
        match lp.as_ref() {
            Loop::Loop(block, _) => {
                assert_eq!(block.statements.len(), 1);
                assert!(matches!(&block.statements[0], Statement::Break(Some(_))));
            }
            other => panic!("expected Loop::Loop, got {:?}", other),
        }
    }
}

// ===========================================================================
// Nested control flow: expression within expression
// ===========================================================================

#[test]
fn match_nested_in_function_body_with_if() {
    // Test that match and if can coexist in the same function body
    let input = r#"main = ->
    x = match 1
        0 => "a"
        _ => "b"
    if true then x else "c"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let stmts = &f.lambda.body.statements;
    assert_eq!(stmts.len(), 2);

    // First statement: assignment with match on RHS
    let assign = as_assign(&stmts[0]);
    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Match(m) if m.arms.len() == 2),
        "RHS should be a match with 2 arms, got {:?}",
        pe.operand
    );

    // Second statement: if expression that uses the match result
    let if_expr_stmt = as_expr(&stmts[1]);
    let if_pe = as_primary(if_expr_stmt);
    assert!(
        matches!(&if_pe.operand, Operand::If(_)),
        "second stmt should be an If"
    );
}

#[test]
fn if_inside_match_arm() {
    let input = r#"main = ->
    match 1
        1 => if true then "yes" else "no"
        _ => "default"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        // First arm body should contain an If
        let arm0_expr = as_expr(&m.arms[0].body.statements[0]);
        let arm0_pe = as_primary(arm0_expr);
        assert!(
            matches!(&arm0_pe.operand, Operand::If(_)),
            "arm 0 should be an If"
        );
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn for_loop_body_contains_if_with_break() {
    let input = r#"main = ->
    for i in [1, 2, 3]
        if i > 2 then break i
    0
"#;
    let p = parse(input);
    let f = first_fn(&p);
    // First statement should be a loop expression
    let loop_expr = as_expr(&f.lambda.body.statements[0]);
    let loop_pe = as_primary(loop_expr);

    if let Operand::Loop(lp) = &loop_pe.operand {
        match lp.as_ref() {
            Loop::For(pattern, iter_expr, body, _) => {
                // Pattern should be `i`
                assert!(matches!(&pattern.kind, PatternKind::Ident(ip) if ip.name.name == "i"));
                // Iterator should be an array literal
                assert!(
                    matches!(as_operand(iter_expr), Operand::Literal(lit) if matches!(&lit.kind, LiteralKind::Array(_)))
                );
                // Body should contain an if expression
                assert_eq!(body.statements.len(), 1);
                let body_pe = as_primary(as_expr(&body.statements[0]));
                assert!(matches!(&body_pe.operand, Operand::If(_)));
            }
            other => panic!("expected For loop, got {:?}", other),
        }
    } else {
        panic!("expected Loop operand");
    }
}

#[test]
fn while_condition_is_binop() {
    let input = r#"main = ->
    x = 0
    while x < 10
        x = x + 1
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    // Statement index 1 is the while loop
    let while_expr = as_expr(&f.lambda.body.statements[1]);
    let while_pe = as_primary(while_expr);

    if let Operand::Loop(lp) = &while_pe.operand {
        match lp.as_ref() {
            Loop::While(cond, body, _) => {
                // Condition should be a binop `x < 10`
                assert!(
                    matches!(&cond.expression, Expression::BinopExpr(_, _, _)),
                    "while condition should be a BinopExpr",
                );
                let (lhs, op, rhs) = as_binop(&cond.expression);
                assert_eq!(unary_ident_name(lhs), "x");
                assert_eq!(op, "<");
                assert_eq!(lit_number(rhs), 10);

                // Body should contain an assignment `x = x + 1`
                assert_eq!(body.statements.len(), 1);
                assert!(matches!(&body.statements[0], Statement::Assignment(_)));
            }
            other => panic!("expected While loop, got {:?}", other),
        }
    } else {
        panic!("expected Loop operand");
    }
}

// ===========================================================================
// Deeply nested constructs
// ===========================================================================

#[test]
fn nested_parenthesized_expressions_preserve_structure() {
    // `((1 + 2))` → Expression(Expression(BinopExpr(1, +, 2)))
    let p = parse("main = -> ((1 + 2))\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    // Outer should be a parenthesized expression
    let pe = as_primary(expr);
    match &pe.operand {
        Operand::Expression(inner) => {
            // Inner should also be parenthesized or directly the binop
            let inner_pe = as_primary(inner);
            match &inner_pe.operand {
                Operand::Expression(innermost) => {
                    // Innermost should be `1 + 2`
                    let (lhs, op, rhs) = as_binop(innermost);
                    assert_eq!(op, "+");
                    assert_eq!(unary_lit_number(lhs), 1);
                    assert_eq!(lit_number(rhs), 2);
                }
                other => panic!("expected nested Expression, got {:?}", other),
            }
        }
        other => panic!("expected Expression (parens), got {:?}", other),
    }
}

#[test]
fn nested_if_else_chain_structure() {
    let input = r#"main = x ->
    if x > 10 then 3
    else if x > 5 then 2
    else 1
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::If(outer_if) = &pe.operand {
        // Outer if: condition is `x > 10`, then yields 3
        let (lhs, op, _) = as_binop(&outer_if.condition.expression);
        assert_eq!(unary_ident_name(lhs), "x");
        assert_eq!(op, ">");

        // Else should be another If
        match &outer_if.else_ {
            Some(Else::If(inner_if)) => {
                // Inner if: condition is `x > 5`, then yields 2
                let (lhs2, _, _) = as_binop(&inner_if.condition.expression);
                assert_eq!(unary_ident_name(lhs2), "x");

                // Else should be a plain block yielding 1
                match &inner_if.else_ {
                    Some(Else::Block(block)) => {
                        assert_eq!(lit_number(as_expr(&block.statements[0])), 1);
                    }
                    other => panic!("expected Else::Block for innermost else, got {:?}", other),
                }
            }
            other => panic!("expected Else::If, got {:?}", other),
        }
    } else {
        panic!("expected If operand");
    }
}

#[test]
fn deeply_nested_match_with_complex_patterns() {
    let input = r#"main = ->
    match (1, (2, 3))
        (0, _) => "zero"
        (a, (b, c)) => match b
            2 => "found two"
            _ => "nope"
        _ => "default"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 3);

        // Second arm: pattern is (a, (b, c)), body contains another match
        let arm1 = &m.arms[1];
        match &arm1.pattern.kind {
            PatternKind::Tuple(elems) => {
                assert_eq!(elems.len(), 2);
                // Second element should be a nested tuple pattern
                match &elems[1].kind {
                    PatternKind::Tuple(inner_elems) => assert_eq!(inner_elems.len(), 2),
                    other => panic!("expected nested tuple pattern, got {:?}", other),
                }
            }
            other => panic!("expected tuple pattern, got {:?}", other),
        }

        // Arm 1 body should be a nested match
        let arm1_expr = as_expr(&arm1.body.statements[0]);
        let arm1_pe = as_primary(arm1_expr);
        assert!(
            matches!(&arm1_pe.operand, Operand::Match(inner_m) if inner_m.arms.len() == 2),
            "inner match should have 2 arms"
        );
    } else {
        panic!("expected Match operand");
    }
}

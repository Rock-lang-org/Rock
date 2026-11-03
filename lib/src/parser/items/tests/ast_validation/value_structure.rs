use super::*;

// ===========================================================================
// Lambda structure
// ===========================================================================

#[test]
fn lambda_params_are_patterns() {
    let p = parse("f = (a, b), c -> a\n");
    let f = first_fn(&p);

    assert_eq!(f.lambda.parameters.len(), 2);
    // First param is a tuple pattern
    assert!(matches!(&f.lambda.parameters[0].kind, PatternKind::Tuple(elems) if elems.len() == 2));
    // Second param is a simple ident
    assert!(matches!(
        &f.lambda.parameters[1].kind,
        PatternKind::Ident(_)
    ));
}

#[test]
fn lambda_body_expression_vs_block() {
    // Inline body: `f = x -> x + 1`
    let p = parse("f = x -> x + 1\n");
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 1);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Expression(Expression::BinopExpr(_, _, _))
    ));
}

#[test]
fn nested_lambda_in_assignment() {
    let input = r#"main = ->
    compose = f, g, x -> f (g x)
    compose
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // The RHS should be a lambda
    let pe = as_primary(&assign.rhs);
    if let Operand::LambdaDecl(lambda) = &pe.operand {
        assert_eq!(lambda.parameters.len(), 3);
        // Body should have one expression: `f (g x)`
        assert_eq!(lambda.body.statements.len(), 1);
        let body_expr = as_expr(&lambda.body.statements[0]);
        let body_pe = as_primary(body_expr);
        // `f` should have an argument which is `(g x)` — a parenthesized expression
        let secs = body_pe
            .secondaries
            .as_ref()
            .expect("f should have arguments");
        assert_eq!(secs.len(), 1);
        assert!(matches!(&secs[0], SecondaryExpr::Arguments(args) if args.len() == 1));
    } else {
        panic!("expected LambdaDecl, got {:?}", pe.operand);
    }
}

#[test]
fn inline_curried_lambda() {
    let input = r#"main = ->
    inc = x ~> x + 1
    inc
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    if let Operand::LambdaDecl(lambda) = &pe.operand {
        assert_eq!(lambda.arrow_kind, LambdaArrowKind::Curried);
        assert_eq!(lambda.parameters.len(), 1);
    } else {
        panic!("expected LambdaDecl, got {:?}", pe.operand);
    }
}

// ===========================================================================
// Pattern depth in match
// ===========================================================================

#[test]
fn match_guard_is_proper_expression() {
    let input = r#"main = ->
    match x
        n if n > 0 && n < 100 => n
        _ => 0
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        // First arm should have a guard that is a BinopExpr (n > 0 && n < 100)
        let guard = m.arms[0]
            .condition
            .as_ref()
            .expect("arm 0 should have guard");
        // The guard is `n > 0 && n < 100` which is a chain of binops
        assert!(
            matches!(guard, Expression::BinopExpr(_, _, _)),
            "guard should be a BinopExpr"
        );
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn match_arm_with_binding_at_sign() {
    let input = r#"main = ->
    match (1, 2)
        foo @ (a, b) => foo
        _ => (0, 0)
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        let arm0 = &m.arms[0];
        // Should have a binding named "foo"
        assert!(arm0.pattern.binding.is_some());
        assert_eq!(arm0.pattern.binding.as_ref().unwrap().name, "foo");
        // Kind should be a tuple
        assert!(matches!(&arm0.pattern.kind, PatternKind::Tuple(elems) if elems.len() == 2));
    } else {
        panic!("expected Match operand");
    }
}

// ===========================================================================
// Array literal contents
// ===========================================================================

#[test]
fn array_literal_elements_are_correct() {
    let p = parse("main = -> [10, 20, 30]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Literal(lit) = &pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 3);
            assert_eq!(lit_number(&arr.elements[0]), 10);
            assert_eq!(lit_number(&arr.elements[1]), 20);
            assert_eq!(lit_number(&arr.elements[2]), 30);
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand");
    }
}

#[test]
fn array_of_expressions() {
    let p = parse("main = -> [1 + 2, 3 * 4]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Literal(lit) = &pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 2);
            // Each element should be a BinopExpr
            assert!(matches!(&arr.elements[0], Expression::BinopExpr(_, _, _)));
            assert!(matches!(&arr.elements[1], Expression::BinopExpr(_, _, _)));
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand");
    }
}

// ===========================================================================
// Struct instantiation field structure
// ===========================================================================

#[test]
fn struct_instantiation_fields_are_correct() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    p = Point x: 10, y: 20
    p
"#;
    let p = parse(input);
    let f = find_fn(&p, "main");
    let assign = as_assign(&f.lambda.body.statements[0]);
    let pe = as_primary(&assign.rhs);

    if let Operand::Instance(inst) = &pe.operand {
        assert_eq!(inst.fields.len(), 2);
        // Fields are in a BTreeMap so they come out alphabetically
        let x_expr = inst
            .fields
            .get(&Ident {
                name: "x".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have x");
        let y_expr = inst
            .fields
            .get(&Ident {
                name: "y".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have y");
        assert_eq!(lit_number(x_expr), 10);
        assert_eq!(lit_number(y_expr), 20);
    } else {
        panic!("expected Instance operand, got {:?}", pe.operand);
    }
}

#[test]
fn struct_field_with_expression_value() {
    let input = r#"struct Foo
    x: Int

main = ->
    p = Foo x: 1 + 2
    p
"#;
    let p = parse(input);
    let f = find_fn(&p, "main");
    let assign = as_assign(&f.lambda.body.statements[0]);
    let pe = as_primary(&assign.rhs);

    if let Operand::Instance(inst) = &pe.operand {
        let x_expr = inst
            .fields
            .get(&Ident {
                name: "x".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have x");
        // x's value should be a BinopExpr: 1 + 2
        assert!(
            matches!(x_expr, Expression::BinopExpr(_, _, _)),
            "field value should be a BinopExpr"
        );
    } else {
        panic!("expected Instance operand");
    }
}

// ===========================================================================
// Self ident (@) structure
// ===========================================================================

#[test]
fn self_ident_desugars_correctly() {
    let input = r#"struct Foo
    x: Int

impl Foo
    get = -> @x
"#;
    let p = parse(input);
    let i = find_impl(&p, "Foo");
    let get = i
        .methods
        .get(&Ident {
            name: "get".to_string(),
            span: crate::lexer::Span::test(),
        })
        .expect("should have get method");
    let body_expr = as_expr(&get.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    assert!(
        matches!(&pe.operand, Operand::SelfIdent(ident) if ident.name == "x"),
        "expected SelfIdent(x), got {:?}",
        pe.operand
    );
}

// ===========================================================================
// Expression with type annotation
// ===========================================================================

#[test]
fn expression_with_type_annotation_structure() {
    let input = r#"main = ->
    x: Int = 42
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // LHS should be a pattern with type annotation
    match &assign.lhs {
        AssignmentLHS::Pattern {
            pattern,
            type_annotation,
        } => {
            assert!(matches!(&pattern.kind, PatternKind::Ident(ip) if ip.name.name == "x"));
            assert!(type_annotation.is_some());
            let ty = type_annotation.as_ref().unwrap();
            match ty {
                ParseType::Type(inner) => assert_eq!(inner.name, "Int"),
                other => panic!("expected Type annotation, got {:?}", other),
            }
        }
        other => panic!("expected Pattern LHS, got {:?}", other),
    }
}

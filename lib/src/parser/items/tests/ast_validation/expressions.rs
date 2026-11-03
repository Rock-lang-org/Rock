use super::*;

// ===========================================================================
// Test: Assignments and patterns
// ===========================================================================

#[test]
fn simple_assignment() {
    let input = "main = ->\n    x = 42\n    x\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 2);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Assignment(_),
    ));
}

#[test]
fn tuple_pattern_assignment() {
    let input = "main = ->\n    (a, b) = (1, 2)\n    a\n";
    validate(input);
}

#[test]
fn array_destructuring_assignment() {
    let input = r#"main = ->
    x = [1, 2, 3]
    [first, ..rest] = x
    first
"#;
    validate(input);
}

#[test]
fn typed_assignment() {
    let input = r#"main = ->
    x: Int = 42
    x
"#;
    validate(input);
}

// ===========================================================================
// Test: Expressions
// ===========================================================================

#[test]
fn binary_expression() {
    let input = "main = -> 1 + 2\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    let stmt = &f.lambda.body.statements[0];
    assert!(
        matches!(stmt, Statement::Expression(Expression::BinopExpr(_, _, _))),
        "Expected binary expression, got {:?}",
        stmt,
    );
}

#[test]
fn chained_binary_expressions() {
    let input = "main = -> 1 + 2 * 3\n";
    validate(input);
}

#[test]
fn function_call() {
    let input = r#"foo = x -> x

main = -> foo 42
"#;
    validate(input);

    let p = parse(input);
    let f = find_fn(&p, "main");
    let stmt = &f.lambda.body.statements[0];
    if let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe))) = stmt {
        assert!(
            pe.secondaries.is_some(),
            "Function call should have secondaries (arguments)"
        );
        if let Some(secs) = &pe.secondaries {
            assert!(
                matches!(secs[0], SecondaryExpr::Arguments(_)),
                "First secondary should be Arguments",
            );
        }
    } else {
        panic!("Expected primary expression");
    }
}

#[test]
fn method_chaining() {
    let input = r#"main = ->
    chained = foo.bar.baz
    chained
"#;
    validate(input);
}

#[test]
fn index_expression() {
    let input = r#"main = ->
    arr = [1, 2, 3]
    arr[0]
"#;
    validate(input);
}

#[test]
fn nested_function_calls() {
    let input = r#"foo = x -> x
bar = x -> x

main = -> foo (bar 42)
"#;
    validate(input);
}

// ===========================================================================
// Test: Literals
// ===========================================================================

#[test]
fn integer_literal() {
    let input = "main = -> 42\n";
    validate(input);
}

#[test]
fn float_literal() {
    let input = "main = -> 3.14\n";
    validate(input);
}

#[test]
fn string_literal() {
    let input = "main = -> \"hello world\"\n";
    validate(input);
}

#[test]
fn bool_literals() {
    let input = r#"main = ->
    a = true
    b = false
    a
"#;
    validate(input);
}

#[test]
fn array_literal() {
    let input = "main = -> [1, 2, 3]\n";
    validate(input);
}

#[test]
fn nested_array_literal() {
    let input = "main = -> [[1, 2], [3, 4]]\n";
    validate(input);
}

#[test]
fn char_literal() {
    let input = "main = -> 'a'\n";
    validate(input);
}

#[test]
fn tuple_literal() {
    let input = "main = -> (1, 2, 3)\n";
    validate(input);
}

// ===========================================================================
// Test: Unary operators
// ===========================================================================

#[test]
fn unary_negation() {
    let input = "main = -> -1\n";
    validate(input);
}

#[test]
fn unary_not() {
    let input = "main = -> !true\n";
    validate(input);
}

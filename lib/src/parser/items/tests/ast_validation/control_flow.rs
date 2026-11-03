use super::*;

// ===========================================================================
// Test: If expressions
// ===========================================================================

#[test]
fn if_then_else_valid() {
    let input = "main = ->\n    if true then 1 else 2\n";
    validate(input);
}

#[test]
fn if_then_multiline_valid() {
    let input = r#"main = ->
    x = if true
        then 1
        else 2
    x
"#;
    validate(input);
}

#[test]
fn if_else_if_chain_valid() {
    let input = r#"main = x ->
    if x > 10 then 3
    else if x > 5 then 2
    else 1
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // The body should have one statement (the if expression)
    assert_eq!(f.lambda.body.statements.len(), 1);
}

#[test]
fn if_let_valid() {
    let input = r#"enum Result T, E
    Ok T
    Err E

main = ->
    result_val: Result Int, String = Result::Ok 42
    if_let_test = if Ok x = result_val
        then x
        else 0
    if_let_test
"#;
    validate(input);
}

// ===========================================================================
// Test: Match expressions
// ===========================================================================

#[test]
fn match_basic_valid() {
    let input = r#"main = ->
    match 1
        0 => "zero"
        1 => "one"
        _ => "other"
"#;
    validate(input);
}

#[test]
fn match_with_guards() {
    let input = r#"main = ->
    match 5
        n if n > 10 => "big"
        n if n > 0 => "positive"
        _ => "non-positive"
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // Walk to the match expression
    let stmt = &f.lambda.body.statements[0];
    if let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe))) = stmt {
        if let Operand::Match(m) = &pe.operand {
            assert_eq!(m.arms.len(), 3);
            // First two arms should have guards
            assert!(m.arms[0].condition.is_some(), "Arm 0 should have a guard");
            assert!(m.arms[1].condition.is_some(), "Arm 1 should have a guard");
            // Last arm (wildcard) has no guard
            assert!(
                m.arms[2].condition.is_none(),
                "Arm 2 should NOT have a guard"
            );
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected expression statement");
    }
}

#[test]
fn match_tuple_destructuring() {
    let input = r#"main = ->
    match (1, 2)
        (0, _) => "zero first"
        (_, 0) => "zero second"
        (a, b) => "both"
"#;
    validate(input);
}

#[test]
fn match_array_destructuring() {
    let input = r#"main = ->
    match [1, 2, 3]
        [first, ..rest] => first
        _ => 0
"#;
    validate(input);
}

#[test]
fn match_with_binding() {
    let input = r#"main = ->
    match (1, 2, 3)
        foo @ (x, y, z) if x > 0 => foo
        _ => (0, 0, 0)
"#;
    validate(input);
}

// ===========================================================================
// Test: Loop constructs
// ===========================================================================

#[test]
fn for_loop_valid() {
    let input = r#"main = ->
    for item in [1, 2, 3, 4, 5]
        item
    0
"#;
    validate(input);
}

#[test]
fn while_loop_valid() {
    let input = r#"main = ->
    counter = 0
    while counter < 10
        counter = counter + 1
    counter
"#;
    validate(input);
}

#[test]
fn loop_with_break_valid() {
    let input = r#"main = ->
    loop_result = loop
        if true then break 42
        continue
    loop_result
"#;
    validate(input);
}

#[test]
fn for_with_break_continue() {
    let input = r#"main = ->
    for item in [1, 2, 3, 4, 5]
        if item > 3 then break item
    0
"#;
    validate(input);
}

#[test]
fn while_with_break_continue() {
    let input = r#"main = ->
    counter = 0
    while counter < 10
        counter = counter + 1
        if counter == 5 then continue
        if counter == 8 then break
    counter
"#;
    validate(input);
}

// ===========================================================================
// Test: Return, break, continue
// ===========================================================================

#[test]
fn return_statement() {
    let input = "main = ->\n    return 42\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Return(Some(_)),
    ));
}

#[test]
fn break_with_value() {
    let input = r#"main = ->
    loop
        break 42
"#;
    validate(input);
}

#[test]
fn continue_statement() {
    let input = r#"main = ->
    loop
        continue
"#;
    // continue without a value should also pass the visitor walk
    let config = Config::default();
    let program = parse_string(input, &config).unwrap();
    let mut validator = AstValidator::new();
    program.visit(&mut validator);
    // We don't flag continue/break, just check it parsed correctly
    assert!(
        parse_string(input, &config).is_ok(),
        "Should parse continue",
    );
}

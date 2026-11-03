use super::*;

// ===========================================================================
// Test: Complex/realistic programs
// ===========================================================================

#[test]
fn realistic_program_with_structs_and_methods() {
    let input = r#"struct Point
    x: Int
    y: Int

impl Point
    add = other ->
        Point
            x: @x + other.x
            y: @y + other.y

main = ->
    a = Point x: 1, y: 2
    b = Point x: 3, y: 4
    c = a.add b
    c
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.structs, 1);
    assert_eq!(c.impls, 1);
    assert_eq!(c.functions, 1);
}

#[test]
fn realistic_enum_with_match() {
    let input = r#"enum Shape
    Circle Int
    Rectangle Int, Int
    Triangle Int, Int, Int

area = shape ->
    match shape
        Circle r => r * r * 3
        Rectangle w, h => w * h
        Triangle a, b, c => a
"#;
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Shape");
    assert_eq!(e.variants.len(), 3);

    let f = find_fn(&p, "area");
    assert_eq!(f.lambda.parameters.len(), 1);
}

#[test]
fn realistic_trait_implementation() {
    let input = r#"trait Printable
    print: () -> String

struct Dog
    name: String

impl Printable for Dog
    print = -> @name

main = ->
    d = Dog name: "Rex"
    d.print!
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.traits, 1);
    assert_eq!(c.structs, 1);
    assert_eq!(c.impls, 1);
    assert_eq!(c.functions, 1);
}

#[test]
fn realistic_higher_order_functions() {
    let input = r#"apply = f, x -> f x

compose = f, g, x -> f (g x)

main = ->
    inc = x -> x + 1
    double = x -> x * 2
    result = compose inc, double, 5
    result
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);

    // compose should have 3 params
    let compose = find_fn(&p, "compose");
    assert_eq!(compose.lambda.parameters.len(), 3);
}

#[test]
fn realistic_nested_control_flow() {
    let input = r#"main = ->
    result = 0
    for i in [1, 2, 3, 4, 5]
        if i > 3
            then break i
        result = result + i
    result
"#;
    validate(input);
}

#[test]
fn realistic_complex_match() {
    let input = r#"main = ->
    match_test = match (1, "test", [1, 2, 3])
        (0, _, _) => "zero"
        (1, str, arr) if str == "test" => "matched with guard"
        foo @ (x, y, z) if x > 0 => "bound pattern"
        (a, b, [first, ..rest]) => "array destructure"
        _ => "default"
    match_test
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);

    // Should have 2 statements: assignment + expression
    assert_eq!(f.lambda.body.statements.len(), 2);
}

#[test]
fn realistic_lambda_in_expression() {
    let input = r#"main = ->
    x = (a -> a + 1) 5
    x
"#;
    validate(input);
}

#[test]
fn unparenthesized_lambda_after_infix_operator() {
    validate(
        r#"main = ->
    source <!> map_error >>= value -> consume value
"#,
    );
}

// ===========================================================================
// Complex real-world-ish programs: deep structural checks
// ===========================================================================

#[test]
fn realistic_recursive_style_program_structure() {
    let input = r#"map = f, list ->
    match list
        [] => []
        [head, ..tail] => [f head] + map f, tail
"#;
    let p = parse(input);
    let f = find_fn(&p, "map");

    // Should have 2 params
    assert_eq!(f.lambda.parameters.len(), 2);

    // Body is a match expression
    let body_expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 2);

        // First arm: pattern is empty array `[]`
        assert!(matches!(&m.arms[0].pattern.kind, PatternKind::Array(elems) if elems.is_empty()));

        // Second arm: pattern is `[head, ..tail]`
        match &m.arms[1].pattern.kind {
            PatternKind::Array(elems) => {
                assert_eq!(elems.len(), 2);
                assert!(matches!(&elems[0], ArrayPattern::Pattern(_)));
                assert!(matches!(&elems[1], ArrayPattern::Rest(_)));
            }
            other => panic!("expected array pattern, got {:?}", other),
        }

        // Second arm body should be a BinopExpr with `+`
        let arm1_expr = as_expr(&m.arms[1].body.statements[0]);
        let (_, op, _) = as_binop(arm1_expr);
        assert_eq!(op, "+", "second arm should use + to concat arrays");
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn enum_match_exhaustive_pattern_structure() {
    let input = r#"enum Direction
    North
    South
    East
    West

to_string = dir ->
    match dir
        North => "N"
        South => "S"
        East => "E"
        West => "W"
"#;
    let p = parse(input);
    let e = find_enum(&p, "Direction");
    assert_eq!(e.variants.len(), 4);

    let f = find_fn(&p, "to_string");
    let body_expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 4, "should have one arm per variant");
        // Each arm pattern should be an ident matching a variant name
        let arm_names: Vec<&str> = m
            .arms
            .iter()
            .map(|arm| match &arm.pattern.kind {
                PatternKind::Ident(ip) => ip.name.name.as_str(),
                PatternKind::Instance(ip) => match &ip.name.path[0] {
                    IdentOrType::Ident(id) => id.name.as_str(),
                    IdentOrType::Type(ty) => match ty {
                        ParseType::Type(inner) => inner.name.as_str(),
                        _ => panic!("unexpected type in pattern"),
                    },
                },
                other => panic!("expected ident pattern, got {:?}", other),
            })
            .collect();
        assert_eq!(arm_names, vec!["North", "South", "East", "West"]);
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn multiple_statements_structure_preserved() {
    let input = r#"main = ->
    a = 1
    b = a + 2
    c = if b > 2 then b else 0
    [a, b, c]
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let stmts = &f.lambda.body.statements;

    assert_eq!(stmts.len(), 4);
    // stmt 0: Assignment a = 1
    assert!(matches!(&stmts[0], Statement::Assignment(_)));
    // stmt 1: Assignment b = a + 2
    let assign_b = as_assign(&stmts[1]);
    assert!(matches!(&assign_b.rhs, Expression::BinopExpr(_, _, _)));
    // stmt 2: Assignment c = if ...
    let assign_c = as_assign(&stmts[2]);
    assert!(matches!(as_operand(&assign_c.rhs), Operand::If(_)));
    // stmt 3: Expression [a, b, c]
    let last_pe = as_primary(as_expr(&stmts[3]));
    if let Operand::Literal(lit) = &last_pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 3);
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand for last statement");
    }
}
